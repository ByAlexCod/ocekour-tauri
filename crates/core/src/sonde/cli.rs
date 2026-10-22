//! Ligne de commande de la sonde (`ocekour-headless sonde …`, aussi `Ocekour.exe sonde …`).

use super::config::{self, CONFIG_FILE};
use super::{server, service};
use std::path::{Path, PathBuf};

pub const USAGE: &str = "Sonde Ocekour : expose en lecture seule des logs de cette machine aux postes Ocekour.

  sonde init [--dossier=<d>] [--ecoute=<adresse:port>] [--nom=<nom>]
                                   crée sonde.yaml, le certificat et le jeton
  sonde [--config=<sonde.yaml>]    démarre la sonde (premier plan, Ctrl+C pour arrêter)
  sonde verifier [--config=…]      relit la configuration et teste chaque source
  sonde statut [--config=…]        postes abonnés en ce moment
  sonde jeton renouveler [--config=…]
                                   nouveau jeton (l'ancien est refusé aussitôt)
  sonde service installer [--config=…] [--utilisateur=<compte>]
  sonde service desinstaller       service Windows ou unité systemd

Par défaut, --config = sonde.yaml du dossier courant.";

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(&format!("--{name}=")))
}

fn config_path(args: &[String]) -> PathBuf {
    option(args, "config").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(CONFIG_FILE))
}

fn words(args: &[String]) -> Vec<&str> { args.iter().filter(|a| !a.starts_with("--")).map(String::as_str).collect() }

/// `args` : ce qui suit `sonde`. Renvoie le code de sortie.
pub async fn main(args: &[String]) -> i32 {
    let result = match words(args).as_slice() {
        [] if args.iter().any(|a| a == "--help" || a == "-h") => { println!("{USAGE}"); Ok(()) }
        [] if args.iter().any(|a| a == "--service") => return run_service(config_path(args)),
        [] => return run(&config_path(args)).await,
        ["init"] => init(args),
        ["verifier"] => return verify(&config_path(args)),
        ["statut"] => status(&config_path(args)),
        ["jeton", "renouveler"] => renew(&config_path(args)),
        ["service", "installer"] => service::install(&config_path(args), option(args, "utilisateur")).map(|m| println!("{m}")),
        ["service", "desinstaller"] => service::uninstall().map(|m| println!("{m}")),
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn dir_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn init(args: &[String]) -> anyhow::Result<()> {
    let dir = option(args, "dossier").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let init = config::init(&dir, option(args, "ecoute"), option(args, "nom"))?;
    println!("Sonde créée : {}", init.config.display());
    println!();
    println!("À transmettre aux utilisateurs (Ocekour → Ajouter une connexion → Sonde) :");
    println!("  Adresse    : {}", init.url);
    println!("  Empreinte  : {}", init.fingerprint);
    println!("  Jeton      : {}", init.token);
    println!();
    println!("Le jeton n'est affiché qu'une fois (transmettez-le de façon sûre).");
    println!("Déclarez ensuite les sources dans sonde.yaml, vérifiez avec « sonde verifier »,");
    println!("puis démarrez « sonde » ou installez le service (« sonde service installer »).");
    println!("Pensez à ouvrir le port en entrée sur cette machine.");
    Ok(())
}

fn renew(path: &Path) -> anyhow::Result<()> {
    let token = config::renew_token(path)?;
    println!("Nouveau jeton (l'ancien est refusé, les postes connectés sont déconnectés) :");
    println!("  {token}");
    Ok(())
}

fn verify(path: &Path) -> i32 {
    let loaded = match config::load(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    println!("Sonde « {} » : {} ({})", loaded.name(), loaded.config.ecoute, if loaded.tls.is_some() { "HTTPS" } else { "HTTP local" });
    if let Some((cert, _)) = &loaded.tls {
        match std::fs::read(cert).ok().and_then(|pem| rustls_pemfile::certs(&mut pem.as_slice()).next()).and_then(Result::ok) {
            Some(der) => println!("Empreinte du certificat : {}", config::fingerprint(&der)),
            None => {
                eprintln!("{} : certificat illisible", cert.display());
                return 1;
            }
        }
    }
    if loaded.config.sources.is_empty() { println!("Aucune source déclarée (sources: dans sonde.yaml)."); }
    let mut ok = true;
    for s in &loaded.config.sources {
        let d = server::describe(s);
        let etat = d["etat"].as_str().unwrap_or("?");
        if etat != "ok" { ok = false; }
        let detail = match (d["fichiers"].as_u64(), d["taille"].as_u64()) {
            (Some(n), _) => format!("{n} fichier(s)"),
            (None, Some(size)) => format!("{size} octets"),
            _ => String::new(),
        };
        println!("  {:<4} {:<24} {:<8} {}", if etat == "ok" { "OK" } else { "!!" }, s.id, etat, detail);
    }
    if ok { 0 } else { 1 }
}

fn status(path: &Path) -> anyhow::Result<()> {
    let status = server::read_status(&dir_of(path))?;
    let fresh = status["misAJour"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| chrono::Utc::now().signed_duration_since(t) < chrono::Duration::seconds(30));
    let subs = status["abonnes"].as_array().cloned().unwrap_or_default();
    println!("Sonde « {} » sur {} (démarrée le {}){}", status["nom"].as_str().unwrap_or("?"), status["ecoute"].as_str().unwrap_or("?"),
        status["demarre"].as_str().unwrap_or("?"), if fresh { "" } else { " — état ancien : la sonde est-elle arrêtée ?" });
    println!("{} abonnement(s) en cours", subs.len());
    for s in subs {
        println!("  {:<28} {:<16} {:<20} {:<10} depuis {}  {} ligne(s)",
            s["poste"].as_str().unwrap_or("?"), s["ip"].as_str().unwrap_or("?"),
            format!("{}{}", s["source"].as_str().unwrap_or("?"), s["fichier"].as_str().map(|f| format!(" › {f}")).unwrap_or_default()),
            if s["suivi"] == true { "suivi" } else { "historique" }, s["depuis"].as_str().unwrap_or("?"), s["lignes"]);
    }
    Ok(())
}

async fn until_stopped() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

/// Sonde au premier plan, jusqu'à Ctrl+C (ou l'arrêt du service).
pub async fn run(path: &Path) -> i32 {
    crate::logging::init(&dir_of(path), true);
    match server::start(path, server::Timing::default()).await {
        Ok(running) => {
            eprintln!("Sonde démarrée sur {} (Ctrl+C pour arrêter). Journal : {}", running.addr, crate::logging::log_file().display());
            until_stopped().await;
            running.stop();
            log::info!("Sonde arrêtée.");
            0
        }
        Err(e) => {
            log::error!("Démarrage impossible : {e}");
            eprintln!("Démarrage impossible : {e}");
            1
        }
    }
}

#[cfg(windows)]
fn run_service(path: PathBuf) -> i32 { windows::run(path) }

#[cfg(not(windows))]
fn run_service(_: PathBuf) -> i32 {
    eprintln!("--service : réservé au service Windows (sous Linux, l'unité systemd lance « sonde »).");
    2
}

#[cfg(windows)]
mod windows {
    //! Service Windows : le gestionnaire de services démarre l'exe avec `sonde --service`.
    use super::super::{server, service::WINDOWS_SERVICE};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;
    use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    static CONFIG: once_cell::sync::OnceCell<PathBuf> = once_cell::sync::OnceCell::new();

    define_windows_service!(ffi_service_main, service_main);

    pub fn run(path: PathBuf) -> i32 {
        let _ = CONFIG.set(path);
        match service_dispatcher::start(WINDOWS_SERVICE, ffi_service_main) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Service Windows : {e} (cette option est utilisée par le gestionnaire de services).");
                1
            }
        }
    }

    fn status(state: ServiceState, accept: ServiceControlAccept, code: u32) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(5),
            process_id: None,
        }
    }

    fn service_main(_: Vec<OsString>) {
        let Some(path) = CONFIG.get().cloned() else { return };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(handle) = service_control_handler::register(WINDOWS_SERVICE, handler) else { return };
        crate::logging::init(&super::dir_of(&path), false);
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime tokio");
        let code = runtime.block_on(async {
            match server::start(&path, server::Timing::default()).await {
                Ok(running) => {
                    let _ = handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN, 0));
                    let _ = tokio::task::spawn_blocking(move || rx.recv()).await;
                    running.stop();
                    log::info!("Service arrêté.");
                    0
                }
                Err(e) => {
                    log::error!("Démarrage impossible : {e}");
                    1
                }
            }
        });
        let _ = handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty(), code));
    }
}
