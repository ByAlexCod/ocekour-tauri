//! Ce que la fenêtre Tauri apporte au cœur : événements, boîtes de dialogue, presse-papiers,
//! notifications système.

use ocekour_core::ui::{FileFilter, OpenOptions, Ui};
use once_cell::sync::OnceCell;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};
use tauri_plugin_notification::NotificationExt;

/// Interface Tauri. L'`AppHandle` n'existe qu'une fois Tauri démarré : avant, rien n'est émis.
#[derive(Default)]
pub struct TauriUi {
    handle: OnceCell<AppHandle>,
}

impl TauriUi {
    pub fn attach(&self, handle: AppHandle) { let _ = self.handle.set(handle); }

    fn handle(&self) -> anyhow::Result<&AppHandle> { self.handle.get().ok_or_else(|| anyhow::anyhow!("Fenêtre non prête.")) }

    fn dialog(&self, title: &str, default_path: Option<PathBuf>, filters: &[FileFilter]) -> Option<FileDialogBuilder<tauri::Wry>> {
        let handle = self.handle.get()?;
        let mut builder = handle.dialog().file().set_title(title);
        if let Some(window) = handle.get_webview_window(crate::MAIN_WINDOW) { builder = builder.set_parent(&window); }
        if let Some(path) = default_path {
            match (path.extension().is_some() || !path.is_dir(), path.parent(), path.file_name()) {
                (true, Some(dir), Some(name)) => {
                    builder = builder.set_directory(dir).set_file_name(name.to_string_lossy());
                }
                _ => builder = builder.set_directory(path),
            }
        }
        for f in filters {
            let extensions: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
            builder = builder.add_filter(&f.name, &extensions);
        }
        Some(builder)
    }
}

/// Chemins choisis dans une boîte de dialogue (les URI non locales sont ignorées).
fn local(path: tauri_plugin_dialog::FilePath) -> Option<PathBuf> { path.into_path().ok() }

#[async_trait::async_trait]
impl Ui for TauriUi {
    fn emit(&self, event: &str, payload: Value) {
        if let Some(handle) = self.handle.get() {
            if let Err(e) = handle.emit(event, payload) { log::debug!("Événement {event} non émis : {e}"); }
        }
    }

    fn emit_to(&self, window: &str, event: &str, payload: Value) {
        if let Some(handle) = self.handle.get() {
            if let Err(e) = handle.emit_to(window, event, payload) { log::debug!("Événement {event} non émis : {e}"); }
        }
    }

    async fn save_dialog(&self, title: &str, default_path: PathBuf, filters: Vec<FileFilter>) -> Option<PathBuf> {
        let builder = self.dialog(title, Some(default_path), &filters)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        builder.save_file(move |path| { let _ = tx.send(path.and_then(local)); });
        rx.await.ok().flatten()
    }

    async fn open_dialog(&self, options: OpenOptions) -> Vec<PathBuf> {
        let Some(builder) = self.dialog(&options.title, options.default_path.clone(), &options.filters) else { return vec![] };
        let (tx, rx) = tokio::sync::oneshot::channel::<Vec<PathBuf>>();
        let many = |paths: Option<Vec<tauri_plugin_dialog::FilePath>>| paths.unwrap_or_default().into_iter().filter_map(local).collect();
        let one = |path: Option<tauri_plugin_dialog::FilePath>| path.and_then(local).into_iter().collect();
        match (options.directory, options.multiple) {
            (true, true) => builder.pick_folders(move |p| { let _ = tx.send(many(p)); }),
            (true, false) => builder.pick_folder(move |p| { let _ = tx.send(one(p)); }),
            (false, true) => builder.pick_files(move |p| { let _ = tx.send(many(p)); }),
            (false, false) => builder.pick_file(move |p| { let _ = tx.send(one(p)); }),
        }
        rx.await.unwrap_or_default()
    }

    fn copy_text(&self, text: &str) -> anyhow::Result<()> {
        self.handle()?.clipboard().write_text(text.to_string()).map_err(|e| anyhow::anyhow!("Presse-papiers : {e}"))
    }

    fn read_clipboard(&self) -> anyhow::Result<String> {
        self.handle()?.clipboard().read_text().map_err(|e| anyhow::anyhow!("Presse-papiers : {e}"))
    }

    fn notify(&self, title: &str, body: &str) {
        let Some(handle) = self.handle.get() else { return };
        if let Err(e) = handle.notification().builder().title(title).body(body).show() {
            log::warn!("Notification système impossible : {e}");
        }
    }

    fn downloads_dir(&self) -> PathBuf {
        self.handle
            .get()
            .and_then(|h| h.path().download_dir().ok())
            .unwrap_or_else(|| ocekour_core::ui::NoUi.downloads_dir())
    }
}
