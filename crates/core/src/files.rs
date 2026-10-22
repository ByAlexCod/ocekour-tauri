//! Fichiers de logs du poste : connexion réservée `local`, fichiers récents et favoris
//! (`fichiers-recents.json`, propres au poste), encodage mémorisé par fichier, et fichiers
//! visibles par les agents (liste blanche de `config.yaml`, `fichiers.agents`).

use crate::error::{ErrorCode, SourceError};
use crate::rancher::SourceNode;
use crate::streams::encoding::Encoding;
use crate::streams::file::FileTarget;
use crate::streams::pattern::{FileEntry, MAX_FILES, Pattern, is_pattern};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Id de la connexion des fichiers du poste (jamais écrite dans config.yaml).
pub const LOCAL: &str = "local";
/// Au plus, fichiers récents gardés.
pub const MAX_RECENTS: usize = 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(default)]
    recents: Vec<String>,
    #[serde(default)]
    favorites: Vec<String>,
    /// Encodage choisi par chemin (fichier ou motif).
    #[serde(default)]
    encodings: std::collections::BTreeMap<String, String>,
}

/// Un fichier ou motif de la section « Fichiers locaux ».
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalFile {
    pub path: String,
    pub name: String,
    pub pattern: bool,
    pub favorite: bool,
    /// Fichier (ou au moins un fichier du motif) présent.
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

pub struct LocalFiles {
    file: PathBuf,
    state: Mutex<Stored>,
    /// Fichiers passés au lancement, en attente de l'interface.
    pending: Mutex<Vec<String>>,
}

/// Fichiers à ouvrir dans les arguments de la ligne de commande (chemins absolus, existants).
pub fn paths_in_args(args: &[String], cwd: &Path) -> Vec<String> {
    args.iter()
        .filter(|a| !a.starts_with('-'))
        .map(|a| cwd.join(a))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Nom affiché d'un chemin (dernier segment, motif compris).
pub fn display_name(path: &str) -> String { path.rsplit(['/', '\\']).next().unwrap_or(path).to_string() }

impl LocalFiles {
    pub fn new(file: PathBuf) -> LocalFiles {
        let state = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        LocalFiles { file, state: Mutex::new(state), pending: Mutex::new(Vec::new()) }
    }

    /// Fichiers de la ligne de commande, à ouvrir quand l'interface sera prête.
    pub fn queue(&self, paths: Vec<String>) { self.pending.lock().extend(paths); }

    /// Fichiers en attente (vidés : chacun n'est ouvert qu'une fois).
    pub fn take_pending(&self) -> Vec<String> { std::mem::take(&mut *self.pending.lock()) }

    fn save(&self, state: &Stored) {
        let text = serde_json::to_string_pretty(state).unwrap_or_default();
        if let Err(e) = std::fs::write(&self.file, text) { log::warn!("{} : écriture impossible : {e}", self.file.display()); }
    }

    fn update(&self, change: impl FnOnce(&mut Stored)) -> Vec<LocalFile> {
        let mut state = self.state.lock();
        change(&mut state);
        self.save(&state);
        drop(state);
        self.list()
    }

    /// Favoris puis récents (sans doublon), avec leur état actuel.
    pub fn list(&self) -> Vec<LocalFile> {
        let state = self.state.lock().clone();
        let mut paths: Vec<&String> = state.favorites.iter().collect();
        paths.extend(state.recents.iter().filter(|p| !state.favorites.contains(p)));
        paths.into_iter().map(|p| describe(p, state.favorites.contains(p), state.encodings.get(p).cloned())).collect()
    }

    /// Fichier ouvert : en tête des récents.
    pub fn touch(&self, path: &str) -> Result<Vec<LocalFile>, SourceError> {
        FileTarget::parse(path)?;
        let path = path.trim().to_string();
        Ok(self.update(|s| {
            s.recents.retain(|p| *p != path);
            s.recents.insert(0, path);
            s.recents.truncate(MAX_RECENTS);
        }))
    }

    pub fn set_favorite(&self, path: &str, favorite: bool) -> Result<Vec<LocalFile>, SourceError> {
        FileTarget::parse(path)?;
        let path = path.trim().to_string();
        Ok(self.update(|s| {
            s.favorites.retain(|p| *p != path);
            if favorite { s.favorites.push(path); }
        }))
    }

    /// Retire des récents et des favoris.
    pub fn forget(&self, path: &str) -> Vec<LocalFile> {
        self.update(|s| {
            s.recents.retain(|p| p != path);
            s.favorites.retain(|p| p != path);
            s.encodings.remove(path);
        })
    }

    pub fn set_encoding(&self, path: &str, encoding: Option<&str>) -> Result<Vec<LocalFile>, SourceError> {
        let encoding = match encoding {
            None | Some("") | Some("auto") => None,
            Some(e) => Some(Encoding::parse(e).ok_or_else(|| SourceError::invalid(format!("Encodage inconnu : {e}")))?.as_str().to_string()),
        };
        Ok(self.update(|s| match encoding {
            Some(e) => { s.encodings.insert(path.to_string(), e); }
            None => { s.encodings.remove(path); }
        }))
    }

    pub fn encoding_of(&self, path: &str) -> Option<Encoding> {
        self.state.lock().encodings.get(path).and_then(|e| Encoding::parse(e))
    }
}

fn describe(path: &str, favorite: bool, encoding: Option<String>) -> LocalFile {
    let pattern = is_pattern(path);
    let newest = if pattern {
        Pattern::parse(path).ok().and_then(|p| p.list(1).ok()).and_then(|l| l.into_iter().next())
    } else {
        FileEntry::of(Path::new(path)).ok()
    };
    LocalFile {
        path: path.to_string(),
        name: display_name(path),
        pattern,
        favorite,
        exists: newest.is_some(),
        size: newest.as_ref().map(|f| f.size),
        modified: newest.and_then(|f| f.modified),
        encoding,
    }
}

/// Cible d'un chemin de flux local : `[fichier ou motif, fichier du motif ?]`.
pub fn target_of(path: &[String]) -> Result<FileTarget, SourceError> {
    match path {
        [one] => FileTarget::parse(one),
        [pattern, name] => match FileTarget::parse(pattern)? {
            FileTarget::Pattern(p) => Ok(FileTarget::File(p.file(name)?)),
            FileTarget::File(_) => Err(SourceError::invalid("Chemin local : [fichier] ou [motif, fichier].")),
        },
        _ => Err(SourceError::invalid("Chemin local : [fichier] ou [motif, fichier].")),
    }
}

/// Fichiers d'un motif, comme nœuds de l'arbre.
pub fn browse(path: &[String]) -> Result<Vec<SourceNode>, SourceError> {
    let [pattern] = path else { return Ok(vec![]) };
    let FileTarget::Pattern(p) = FileTarget::parse(pattern)? else { return Ok(vec![]) };
    Ok(p.list(MAX_FILES)?.into_iter().map(file_node).collect())
}

pub fn file_node(f: FileEntry) -> SourceNode {
    SourceNode { id: f.name, kind: "file", expandable: false, health: Some("ok"), size: Some(f.size), modified: f.modified, ..Default::default() }
}

/// Clé de profil d'un fichier : `fichier/<nom sans numéros de rotation ni date>`.
pub fn profile_key(path: &[String]) -> String {
    static DIGITS: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"[-_.]?\d{4}-\d\d-\d\d|[-_.]?\d{8}|\.\d+$|-\d+(\.)").unwrap());
    let name = display_name(path.last().map(String::as_str).unwrap_or_default());
    let name = name.trim_end_matches(".gz").replace("-*", "").replace("_*", "").replace(".*", "").replace('*', "");
    format!("fichier/{}", DIGITS.replace_all(&name, "$1"))
}

// --- Agents : liste blanche --------------------------------------------------------------------

fn canonical(path: &Path) -> Option<PathBuf> { dunce_canonical(path).ok() }

#[cfg(windows)]
fn dunce_canonical(path: &Path) -> std::io::Result<PathBuf> {
    // Sans le préfixe \\?\ de Windows, pour comparer avec les chemins saisis.
    let p = std::fs::canonicalize(path)?;
    let s = p.to_string_lossy();
    Ok(match s.strip_prefix(r"\\?\") { Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest), _ => p })
}

#[cfg(not(windows))]
fn dunce_canonical(path: &Path) -> std::io::Result<PathBuf> { std::fs::canonicalize(path) }

fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { a == b }
}

/// Vérifie qu'un chemin de flux local est autorisé aux agents. Un motif autorisé peut être lu
/// tel quel ; un fichier l'est s'il est listé, ou s'il correspond à un motif listé (chemins
/// réels comparés : liens symboliques et `..` résolus).
pub fn agent_allowed(allowed: &[String], path: &[String]) -> Result<(), SourceError> {
    let refuse = || SourceError::new(ErrorCode::Forbidden, "Fichier non autorisé aux agents : l'utilisateur doit l'ajouter dans Paramètres → Fichiers (fichiers visibles par les agents).");
    let requested = path.first().ok_or_else(refuse)?.trim();
    if path.len() == 1 && is_pattern(requested) {
        return if allowed.iter().any(|a| a.trim() == requested) { Ok(()) } else { Err(refuse()) };
    }
    let file = match target_of(path)? {
        FileTarget::File(f) => f,
        FileTarget::Pattern(_) => return Err(refuse()),
    };
    let real = canonical(&file).ok_or_else(|| SourceError::new(ErrorCode::NotFound, format!("{} : introuvable.", file.display())))?;
    for entry in allowed {
        if is_pattern(entry) {
            let Ok(p) = Pattern::parse(entry) else { continue };
            let Some(dir) = canonical(&p.dir) else { continue };
            let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if real.parent().is_some_and(|parent| same(parent, &dir)) && p.matches(&name) { return Ok(()); }
        } else if canonical(Path::new(entry.trim())).is_some_and(|c| same(&c, &real)) {
            return Ok(());
        }
    }
    Err(refuse())
}

/// Fichiers autorisés aux agents, avec leur état (outil `fichiers_lister`).
pub fn agent_files(allowed: &[String]) -> Vec<LocalFile> { allowed.iter().map(|p| describe(p.trim(), false, None)).collect() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recents_favoris_encodages() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("app.log");
        std::fs::write(&log, "x\n").unwrap();
        let store = dir.path().join("fichiers-recents.json");
        let files = LocalFiles::new(store.clone());
        let p = log.to_string_lossy().to_string();
        let pattern = format!("{}/*.log", dir.path().display());
        for i in 0..12 { files.touch(&format!("{}/f{i}.log", dir.path().display())).unwrap(); }
        files.touch(&p).unwrap();
        let list = files.touch(&pattern).unwrap();
        assert_eq!(list.len(), MAX_RECENTS);
        assert_eq!(list[0].path, pattern);
        assert!(list[0].pattern && list[0].exists && list[0].name == "*.log");
        assert!(list[1].exists && list[1].size == Some(2) && !list[2].exists);
        assert!(files.touch("relatif.log").is_err());

        files.set_favorite(&p, true).unwrap();
        files.set_encoding(&p, Some("cp1252")).unwrap();
        assert!(files.set_encoding(&p, Some("ebcdic")).is_err());
        // Relu depuis le disque.
        let again = LocalFiles::new(store);
        let list = again.list();
        assert_eq!((list[0].path.as_str(), list[0].favorite, list[0].encoding.as_deref()), (p.as_str(), true, Some("windows-1252")));
        assert_eq!(list.iter().filter(|f| f.path == p).count(), 1);
        assert_eq!(again.encoding_of(&p), Some(Encoding::Windows1252));
        let list = again.forget(&p);
        assert!(!list.iter().any(|f| f.path == p));
    }

    #[test]
    fn chemins_et_profils() {
        assert!(target_of(&["/var/log/app.log".into()]).is_ok());
        assert!(matches!(target_of(&["/var/log/app-*.log".into()]).unwrap(), FileTarget::Pattern(_)));
        assert!(matches!(target_of(&["/var/log/app-*.log".into(), "app-1.log".into()]).unwrap(), FileTarget::File(_)));
        assert!(target_of(&["/var/log/app-*.log".into(), "../etc/passwd".into()]).is_err());
        assert!(target_of(&["/var/log/app.log".into(), "x".into()]).is_err());
        assert_eq!(profile_key(&["/l/app.2026-10-22.log".into()]), "fichier/app.log");
        assert_eq!(profile_key(&["/l/app-*.log".into(), "app-20261022.log".into()]), "fichier/app.log");
        assert_eq!(profile_key(&["/l/app.log.3".into()]), "fichier/app.log");
        assert_eq!(profile_key(&["/l/app.log".into()]), "fichier/app.log");
        assert_eq!(profile_key(&["/l/app-*.log".into()]), "fichier/app.log");
    }

    #[cfg(unix)]
    #[test]
    fn liste_blanche_des_agents() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        std::fs::write(logs.join("app-1.log"), "x").unwrap();
        std::fs::write(logs.join("seul.log"), "x").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), logs.join("app-lien.log")).unwrap();
        let d = logs.display().to_string();
        let allowed = vec![format!("{d}/app-*.log"), format!("{d}/seul.log")];
        assert!(agent_allowed(&allowed, &[format!("{d}/app-*.log")]).is_ok());
        assert!(agent_allowed(&allowed, &[format!("{d}/app-1.log")]).is_ok());
        assert!(agent_allowed(&allowed, &[format!("{d}/app-*.log"), "app-1.log".into()]).is_ok());
        assert!(agent_allowed(&allowed, &[format!("{d}/seul.log")]).is_ok());
        // Refusés : autre fichier, `..`, lien symbolique vers l'extérieur, autre motif.
        let err = agent_allowed(&allowed, &[format!("{}/secret.txt", dir.path().display())]).unwrap_err();
        assert_eq!(err.code, ErrorCode::Forbidden);
        assert!(agent_allowed(&allowed, &[format!("{d}/../secret.txt")]).is_err());
        assert!(agent_allowed(&allowed, &[format!("{d}/app-lien.log")]).is_err());
        assert!(agent_allowed(&allowed, &[format!("{d}/*.log")]).is_err());
        assert!(agent_allowed(&[], &[format!("{d}/seul.log")]).is_err());
        assert_eq!(agent_files(&allowed).len(), 2);
    }
}
