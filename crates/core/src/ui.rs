//! Ce que l'interface apporte au cœur : boîtes de dialogue, presse-papiers, navigateur,
//! notifications système, événements vers la fenêtre. Sans interface (mode headless), rien.

use serde_json::Value;
use std::path::PathBuf;

/// Filtre de boîte de dialogue : libellé et extensions (sans point).
#[derive(Debug, Clone)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl FileFilter {
    pub fn new(name: &str, extensions: &[&str]) -> Self {
        FileFilter { name: name.into(), extensions: extensions.iter().map(|e| e.to_string()).collect() }
    }
}

/// Boîte d'ouverture : fichier(s) ou dossier.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    pub title: String,
    pub default_path: Option<PathBuf>,
    pub filters: Vec<FileFilter>,
    pub directory: bool,
    pub multiple: bool,
}

#[async_trait::async_trait]
pub trait Ui: Send + Sync {
    /// Événement vers les fenêtres (`config:changed`, `streams:event`…).
    fn emit(&self, _event: &str, _payload: Value) {}
    /// Événement pour une seule fenêtre (flux, progression S3) ; à défaut, pour toutes.
    fn emit_to(&self, _window: &str, event: &str, payload: Value) { self.emit(event, payload) }
    /// Boîte « Enregistrer sous » ; `None` si annulée.
    async fn save_dialog(&self, _title: &str, _default_path: PathBuf, _filters: Vec<FileFilter>) -> Option<PathBuf> { None }
    /// Boîte d'ouverture ; vide si annulée.
    async fn open_dialog(&self, _options: OpenOptions) -> Vec<PathBuf> { vec![] }
    fn copy_text(&self, _text: &str) -> anyhow::Result<()> { anyhow::bail!("Presse-papiers indisponible sans interface.") }
    fn read_clipboard(&self) -> anyhow::Result<String> { anyhow::bail!("Presse-papiers indisponible sans interface.") }
    fn open_url(&self, url: &str) -> anyhow::Result<()> { open_with_system(url) }
    fn show_in_folder(&self, path: &std::path::Path) -> anyhow::Result<()> { open_with_system(&path.parent().unwrap_or(path).to_string_lossy()) }
    /// Notification système (barre des tâches).
    fn notify(&self, title: &str, body: &str) { log::info!("Notification : {title} — {body}"); }
    fn downloads_dir(&self) -> PathBuf { dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")) }
}

/// Aucune interface (mode headless, tests).
pub struct NoUi;
impl Ui for NoUi {}

/// Ouvre une URL ou un dossier avec l'application par défaut du système.
pub fn open_with_system(target: &str) -> anyhow::Result<()> {
    // Windows : ShellExecute, comme un double-clic. (`explorer <url>` ouvre l'Explorateur de
    // fichiers au lieu du navigateur dès que le lien a des paramètres : `?name=…&scopes=…`.)
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, file) = (wide("open"), wide(target));
        // SAFETY : chaînes terminées par 0, vivantes pendant l'appel ; pas de fenêtre parente.
        let code = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) } as isize;
        // Au-delà de 32 : réussi (convention de ShellExecute).
        if code <= 32 { anyhow::bail!("Ouverture impossible ({target}) : erreur Windows {code}."); }
        Ok(())
    }
    #[cfg(not(windows))]
    open_unix(target)
}

#[cfg(not(windows))]
fn open_unix(target: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = { let mut c = std::process::Command::new("open"); c.arg(target); c };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = { let mut c = std::process::Command::new("xdg-open"); c.arg(target); c };
    cmd.spawn()?;
    Ok(())
}
