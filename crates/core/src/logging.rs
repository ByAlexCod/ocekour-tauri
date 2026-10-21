//! Journal applicatif : `<données>/logs/main.log` (5 Mo, puis `main.old.log`). N'y écrire JAMAIS
//! de secret (token, mot de passe, clé, brouillon de connexion).

use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_SIZE: u64 = 5 * 1024 * 1024;

struct Sink {
    path: PathBuf,
    file: Option<File>,
    size: u64,
}

struct Logger {
    sink: Mutex<Sink>,
    console: bool,
}

static FILE: once_cell::sync::OnceCell<PathBuf> = once_cell::sync::OnceCell::new();

impl Sink {
    fn write(&mut self, line: &str) {
        if self.file.is_none() {
            if let Some(dir) = self.path.parent() { let _ = std::fs::create_dir_all(dir); }
            self.file = OpenOptions::new().create(true).append(true).open(&self.path).ok();
            self.size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        }
        if self.size + line.len() as u64 > MAX_SIZE {
            self.file = None;
            let old = self.path.with_file_name("main.old.log");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&self.path, &old);
            self.file = OpenOptions::new().create(true).append(true).open(&self.path).ok();
            self.size = 0;
        }
        if let Some(f) = &mut self.file {
            if f.write_all(line.as_bytes()).is_ok() { self.size += line.len() as u64; }
        }
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info && !is_noise(metadata.target())
    }
    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) { return; }
        let level = match record.level() {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            _ => "info",
        };
        let line = format!(
            "[{}] [{level}] {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            record.args()
        );
        if self.console { eprint!("{line}"); }
        self.sink.lock().write(&line);
    }
    fn flush(&self) {
        if let Some(f) = &mut self.sink.lock().file { let _ = f.flush(); }
    }
}

/// Journaux internes des bibliothèques (rustls, hyper…) : ignorés.
fn is_noise(target: &str) -> bool {
    !(target.starts_with("ocekour") || target == "app")
}

/// Installe le journal ; `console` = aussi sur la sortie d'erreur (jamais la sortie standard,
/// réservée au protocole MCP en mode stdio).
pub fn init(data_dir: &Path, console: bool) {
    let path = data_dir.join("logs").join("main.log");
    let _ = FILE.set(path.clone());
    let logger = Logger { sink: Mutex::new(Sink { path, file: None, size: 0 }), console };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Chemin du journal.
pub fn log_file() -> PathBuf {
    FILE.get().cloned().unwrap_or_else(|| PathBuf::from("main.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_a_5_mo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.log");
        let mut sink = Sink { path: path.clone(), file: None, size: MAX_SIZE - 3 };
        sink.file = None;
        std::fs::write(&path, vec![b'x'; (MAX_SIZE - 3) as usize]).unwrap();
        sink.write("ligne longue\n");
        assert!(dir.path().join("main.old.log").exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ligne longue\n");
    }
}
