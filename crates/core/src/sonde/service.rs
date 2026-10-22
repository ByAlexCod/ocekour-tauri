//! Installation de la sonde en service : Windows (service « Ocekour Sonde », compte virtuel
//! `NT SERVICE\OcekourSonde`) ou Linux (unité systemd).

use std::path::{Path, PathBuf};
use std::process::Command;

pub const WINDOWS_SERVICE: &str = "OcekourSonde";
pub const SYSTEMD_UNIT: &str = "ocekour-sonde.service";

/// Unité systemd : la sonde tourne sous le compte propriétaire de sonde.yaml, en lecture seule
/// sur tout le système sauf son dossier (journal, état).
pub fn systemd_unit(exe: &Path, config: &Path, user: &str) -> String {
    let dir = config.parent().unwrap_or(Path::new("/"));
    format!(
        "[Unit]
Description=Ocekour Sonde (logs en lecture seule)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=\"{exe}\" sonde --config=\"{config}\"
User={user}
Restart=on-failure
RestartSec=5
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=\"{dir}\"
PrivateTmp=yes

[Install]
WantedBy=multi-user.target
",
        exe = exe.display(),
        config = config.display(),
        dir = dir.display(),
    )
}

fn run(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new(program).args(args).output().map_err(|e| anyhow::anyhow!("{program} : {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() { anyhow::bail!("{program} {} : {}", args.join(" "), text.trim()); }
    Ok(text)
}

fn absolute(config: &Path) -> anyhow::Result<PathBuf> {
    Ok(std::fs::canonicalize(config).map_err(|e| anyhow::anyhow!("{} : {e}", config.display()))?)
}

/// Installe et démarre le service ; renvoie un message pour l'administrateur.
pub fn install(config: &Path, user: Option<&str>) -> anyhow::Result<String> {
    let config = absolute(config)?;
    super::config::load(&config).map_err(|e| anyhow::anyhow!("{e}"))?;
    let exe = std::env::current_exe()?;
    if cfg!(windows) {
        let bin = format!("\"{}\" sonde --service --config=\"{}\"", exe.display(), config.display());
        let account = format!("NT SERVICE\\{WINDOWS_SERVICE}");
        run("sc.exe", &["create", WINDOWS_SERVICE, "binPath=", &bin, "start=", "auto", "DisplayName=", "Ocekour Sonde", "obj=", &account])?;
        let _ = run("sc.exe", &["description", WINDOWS_SERVICE, "Expose en lecture seule des logs aux postes Ocekour."]);
        let _ = run("sc.exe", &["failure", WINDOWS_SERVICE, "reset=", "86400", "actions=", "restart/5000/restart/5000/restart/60000"]);
        // Le compte du service écrit son journal et son état dans le dossier de la sonde.
        let dir = config.parent().unwrap_or(Path::new("."));
        run("icacls", &[&dir.to_string_lossy(), "/grant", &format!("{account}:(OI)(CI)M")])?;
        run("sc.exe", &["start", WINDOWS_SERVICE])?;
        Ok(format!(
            "Service « Ocekour Sonde » installé et démarré (compte {account}).\n\
             Donnez à ce compte la lecture des dossiers de logs exposés, par exemple :\n  icacls \"C:\\chemin\\des\\logs\" /grant \"{account}:(OI)(CI)R\""
        ))
    } else {
        let user = match user {
            Some(u) => u.to_string(),
            None => owner(&config).unwrap_or_else(|| "root".into()),
        };
        if user == "root" || user == "0" {
            anyhow::bail!("La sonde ne doit pas tourner en root : précisez --utilisateur=<compte> (lecture des logs, écriture dans {}).", config.parent().unwrap_or(Path::new("/")).display());
        }
        let unit = systemd_unit(&exe, &config, &user);
        let path = PathBuf::from("/etc/systemd/system").join(SYSTEMD_UNIT);
        std::fs::write(&path, &unit).map_err(|e| anyhow::anyhow!("{} : {e} (lancez la commande avec sudo).\nUnité à installer :\n\n{unit}", path.display()))?;
        run("systemctl", &["daemon-reload"])?;
        run("systemctl", &["enable", "--now", SYSTEMD_UNIT])?;
        Ok(format!("Service {SYSTEMD_UNIT} installé et démarré (compte {user}). Journal : journalctl -u {SYSTEMD_UNIT}"))
    }
}

pub fn uninstall() -> anyhow::Result<String> {
    if cfg!(windows) {
        let _ = run("sc.exe", &["stop", WINDOWS_SERVICE]);
        run("sc.exe", &["delete", WINDOWS_SERVICE])?;
        Ok("Service « Ocekour Sonde » supprimé.".into())
    } else {
        let _ = run("systemctl", &["disable", "--now", SYSTEMD_UNIT]);
        let path = PathBuf::from("/etc/systemd/system").join(SYSTEMD_UNIT);
        if path.exists() { std::fs::remove_file(&path).map_err(|e| anyhow::anyhow!("{} : {e} (sudo ?)", path.display()))?; }
        let _ = run("systemctl", &["daemon-reload"]);
        Ok(format!("Service {SYSTEMD_UNIT} supprimé."))
    }
}

/// Compte propriétaire d'un fichier (uid), pour `User=`.
fn owner(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = std::fs::metadata(path).ok()?.uid();
        let name = Command::new("id").args(["-nu", &uid.to_string()]).output().ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|n| !n.is_empty());
        Some(name.unwrap_or_else(|| uid.to_string()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unite_systemd() {
        let unit = systemd_unit(Path::new("/opt/ocekour/ocekour-headless"), Path::new("/srv/sonde/sonde.yaml"), "logs");
        assert!(unit.contains("ExecStart=\"/opt/ocekour/ocekour-headless\" sonde --config=\"/srv/sonde/sonde.yaml\""));
        assert!(unit.contains("User=logs") && unit.contains("ProtectSystem=strict") && unit.contains("ReadWritePaths=\"/srv/sonde\""));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }
}
