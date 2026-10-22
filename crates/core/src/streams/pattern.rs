//! Motifs de fichiers (`C:\app\logs\app-*.log`) : un dossier fixe, un nom avec jokers. Les
//! fichiers sont listés du plus récent au plus ancien.

use crate::error::{ErrorCode, SourceError};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Au plus, fichiers listés pour un motif (les plus récents).
pub const MAX_FILES: usize = 30;

#[derive(Debug, Clone)]
pub struct Pattern {
    pub dir: PathBuf,
    pub glob: String,
    re: regex::Regex,
}

/// Un fichier d'un motif (ou un fichier seul).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    #[serde(skip)]
    pub path: PathBuf,
    pub size: u64,
    /// ISO 8601.
    pub modified: Option<String>,
    #[serde(skip)]
    pub mtime: Option<SystemTime>,
}

impl FileEntry {
    pub fn of(path: &Path) -> std::io::Result<FileEntry> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() { return Err(std::io::Error::other("ce n'est pas un fichier")); }
        let mtime = meta.modified().ok();
        Ok(FileEntry {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: path.to_path_buf(),
            size: meta.len(),
            modified: mtime.map(iso),
            mtime,
        })
    }
}

pub fn iso(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Le chemin est un motif (jokers dans le nom du fichier).
pub fn is_pattern(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.contains(['*', '?', '['])
}

fn glob_regex(glob: &str) -> Result<regex::Regex, SourceError> {
    let mut out = String::from(if cfg!(windows) { "(?i)^" } else { "^" });
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '[' => {
                let mut class = String::from("[");
                if chars.peek() == Some(&'!') { chars.next(); class.push('^'); }
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == ']' { closed = true; break; }
                    if c == '\\' || c == '[' { class.push('\\'); }
                    class.push(c);
                }
                if !closed { return Err(SourceError::invalid("Motif invalide : « [ » sans « ] ».")); }
                class.push(']');
                out.push_str(&class);
            }
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    out.push('$');
    regex::Regex::new(&out).map_err(|e| SourceError::invalid(format!("Motif invalide : {e}")))
}

impl Pattern {
    /// `dossier/nom-*.log` : jokers `*`, `?`, `[…]` dans le nom seulement (pas de `**`).
    pub fn parse(path: &str) -> Result<Pattern, SourceError> {
        let path = path.trim();
        let cut = path.rfind(['/', '\\']).ok_or_else(|| SourceError::invalid("Motif : chemin absolu attendu (dossier + nom)."))?;
        let (dir, glob) = (&path[..cut.max(1)], &path[cut + 1..]);
        let dir = if cut == 0 { &path[..1] } else { dir };
        if glob.is_empty() { return Err(SourceError::invalid("Motif : nom de fichier manquant.")); }
        if glob.contains("**") { return Err(SourceError::invalid("Motif : « ** » n'est pas pris en charge (un seul dossier).")); }
        if is_pattern(dir) || dir.contains('*') || dir.contains('?') {
            return Err(SourceError::invalid("Motif : les jokers ne sont permis que dans le nom du fichier."));
        }
        let dir = PathBuf::from(dir);
        if !dir.is_absolute() { return Err(SourceError::invalid("Motif : chemin absolu attendu.")); }
        Ok(Pattern { re: glob_regex(glob)?, dir, glob: glob.to_string() })
    }

    pub fn display(&self) -> String { self.dir.join(&self.glob).to_string_lossy().into_owned() }

    pub fn matches(&self, name: &str) -> bool { self.re.is_match(name) }

    /// Fichiers du motif, du plus récent au plus ancien (au plus `limit`).
    pub fn list(&self, limit: usize) -> Result<Vec<FileEntry>, SourceError> {
        let dir = std::fs::read_dir(&self.dir).map_err(|e| io_error(&self.dir, &e))?;
        let mut files: Vec<FileEntry> = dir
            .filter_map(Result::ok)
            .filter(|e| self.matches(&e.file_name().to_string_lossy()))
            .filter_map(|e| FileEntry::of(&e.path()).ok())
            .collect();
        files.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| b.name.cmp(&a.name)));
        files.truncate(limit);
        Ok(files)
    }

    /// Fichier du motif désigné par son nom (jamais un chemin : ni séparateur ni `..`).
    pub fn file(&self, name: &str) -> Result<PathBuf, SourceError> {
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." || name.contains('\0') {
            return Err(SourceError::invalid("Nom de fichier invalide."));
        }
        if !self.matches(name) { return Err(SourceError::new(ErrorCode::NotFound, format!("« {name} » ne correspond pas au motif {}.", self.glob))); }
        Ok(self.dir.join(name))
    }
}

/// Erreur d'accès à un fichier, avec le code adapté.
pub fn io_error(path: &Path, e: &std::io::Error) -> SourceError {
    let p = path.display();
    match e.kind() {
        std::io::ErrorKind::NotFound => SourceError::new(ErrorCode::NotFound, format!("{p} : introuvable.")),
        std::io::ErrorKind::PermissionDenied => SourceError::new(ErrorCode::Forbidden, format!("{p} : accès refusé.")),
        _ => SourceError::invalid(format!("{p} : {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyse_et_liste() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_string_lossy().to_string();
        let p = Pattern::parse(&format!("{d}/app-*.log")).unwrap();
        assert!(p.matches("app-2026-10-22.log") && !p.matches("autre.log") && !p.matches("app-1.log.gz"));
        assert!(Pattern::parse(&format!("{d}/**/*.log")).is_err());
        assert!(Pattern::parse(&format!("{d}/*/app.log")).is_err());
        assert!(Pattern::parse("relatif/*.log").is_err());
        assert!(Pattern::parse(&format!("{d}/app[0-9.log")).is_err());
        let p2 = Pattern::parse(&format!("{d}/app.[0-9]")).unwrap();
        assert!(p2.matches("app.1") && !p2.matches("app.x"));
        assert!(is_pattern("/var/log/a-*.log") && !is_pattern("/var/*/a.log") && !is_pattern("C:\\logs\\a.log"));

        for (i, name) in ["app-1.log", "app-2.log", "app-3.log", "autre.log"].iter().enumerate() {
            let path = dir.path().join(name);
            std::fs::write(&path, "x").unwrap();
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64 * 10);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(t).unwrap();
        }
        let names: Vec<String> = p.list(MAX_FILES).unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["app-3.log", "app-2.log", "app-1.log"]);
        assert_eq!(p.list(2).unwrap().len(), 2);

        assert!(p.file("app-1.log").is_ok());
        for bad in ["../app-1.log", "x/app-1.log", "..\\app-1.log", "autre.log", "", ".."] {
            assert!(p.file(bad).is_err(), "{bad}");
        }
        assert_eq!(p.file("autre.log").unwrap_err().code, ErrorCode::NotFound);
    }
}
