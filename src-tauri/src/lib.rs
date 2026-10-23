//! Ocekour (bureau) : une fenêtre Tauri au-dessus du cœur `ocekour-core`. L'interface ne fait
//! qu'appeler le cœur par une commande unique (`call`, mêmes canaux que la version Electron) et
//! recevoir ses événements ; les modes sans interface (`--mcp`, `--mcp-http`, `--notify`)
//! n'ouvrent aucune fenêtre et ne chargent pas WebView2.

pub mod browser;
mod ui;
pub mod webview2;

use ocekour_core::api::{self, Caller};
use ocekour_core::app::{App, AppOptions};
use ocekour_core::headless;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const MAIN_WINDOW: &str = "main";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Canal unique entre l'interface et le cœur.
#[tauri::command]
async fn call<R: tauri::Runtime>(app: tauri::State<'_, Arc<App>>, window: tauri::WebviewWindow<R>, channel: String, args: Vec<Value>) -> Result<Value, String> {
    api::call(&app, &Caller { window: window.label().to_string() }, &channel, args).await
}

/// Commandes exposées à l'interface (une seule : `call`).
pub fn invoke_handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![call]
}

/// L'application n'affiche que son propre front : aucune navigation vers l'extérieur
/// (les liens s'ouvrent dans le navigateur du système via le cœur).
pub fn allowed_navigation(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" | "asset" => true,
        // Windows sert le front sur http://tauri.localhost ; en développement, serveur Vite local.
        "http" | "https" => match url.host_str() {
            Some("tauri.localhost") => true,
            Some("localhost") => cfg!(debug_assertions),
            _ => false,
        },
        _ => false,
    }
}

/// Point d'entrée commun : mode sans interface si demandé, sinon la fenêtre.
pub fn main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        attach_console();
        println!("Ocekour {VERSION}");
        return 0;
    }
    if args.first().map(String::as_str) == Some("sonde") {
        attach_console();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime tokio");
        return runtime.block_on(ocekour_core::sonde::cli::main(&args[1..]));
    }
    let dir = headless::data_dir(&args);
    let mode = headless::parse_mode(&args);
    // Fenêtre : WebView2 (Tauri), vérifié avant de lancer le moindre fil ; s'il n'est pas
    // utilisable, Edge en mode application (`--navigateur` pour le forcer).
    let mut edge: Option<std::path::PathBuf> = None;
    if mode.is_none() {
        let forced = args.iter().any(|a| a == "--navigateur");
        #[cfg(windows)]
        let missing = !forced && webview2::prepare().is_err();
        #[cfg(not(windows))]
        let missing = false;
        if forced || missing {
            match browser::find_browser() {
                Some(path) => edge = Some(path),
                None => {
                    fatal(webview2::MISSING);
                    return 1;
                }
            }
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime tokio");

    if let Some(mode) = mode {
        attach_console();
        // En mode stdio, rien d'autre que le protocole sur la sortie standard.
        ocekour_core::logging::init(&dir, mode != headless::Mode::Stdio);
        return runtime.block_on(async move {
            match App::new(AppOptions::new(&dir, VERSION, Arc::new(ocekour_core::ui::NoUi))) {
                Ok(app) => headless::run(app, mode).await,
                Err(e) => {
                    eprintln!("Démarrage impossible : {e}");
                    1
                }
            }
        });
    }

    ocekour_core::logging::init(&dir, cfg!(debug_assertions));
    install_panic_hook(&args, edge.is_none());
    // Le fil principal (boucle de la fenêtre) reste dans le contexte tokio : le cœur y lance
    // ses tâches (serveur MCP, notifications) depuis les rappels de Tauri.
    let _guard = runtime.enter();
    // Une seule fois : l'interface intégrée à l'exe sert aux deux modes.
    let context = tauri::generate_context!();
    let files = std::env::current_dir().map(|cwd| ocekour_core::files::paths_in_args(&args, &cwd)).unwrap_or_default();

    if let Some(browser) = edge {
        let ui = Arc::new(browser::BrowserUi::default());
        let app = match App::new(AppOptions::new(&dir, VERSION, ui.clone())) {
            Ok(app) => app,
            Err(e) => {
                fatal(&format!("Démarrage impossible : {e}"));
                return 1;
            }
        };
        app.files.queue(files);
        let assets: browser::Assets = Box::leak(Box::new(context)).assets();
        return runtime.block_on(browser::run(app, ui, assets, browser, dir.join("edge")));
    }

    tauri::async_runtime::set(runtime.handle().clone());
    let ui = Arc::new(ui::TauriUi::default());
    let app = match App::new(AppOptions::new(&dir, VERSION, ui.clone())) {
        Ok(app) => app,
        Err(e) => {
            fatal(&format!("Démarrage impossible : {e}"));
            return 1;
        }
    };
    app.files.queue(files);
    if let Err(e) = run_window(app.clone(), ui, context) {
        // Fenêtre WebView2 impossible : l'interface s'ouvre dans Edge en mode application.
        log::error!("Fenêtre impossible : {e}");
        runtime.block_on(app.shutdown());
        drop(app);
        return fall_back_to_browser(&args, &e.to_string());
    }
    0
}

/// Fenêtre ouverte et page chargée : plus de repli vers le navigateur en cas d'incident.
static WINDOW_LOADED: AtomicBool = AtomicBool::new(false);

/// Relance l'exe en mode navigateur (`--navigateur`) ; message si c'est impossible.
fn fall_back_to_browser(args: &[String], cause: &str) -> i32 {
    let relaunched = browser::find_browser().is_some()
        && std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).args(args).arg("--navigateur").spawn())
            .inspect_err(|e| log::error!("Relance en mode navigateur impossible : {e}"))
            .is_ok();
    if relaunched {
        log::info!("Interface relancée dans le navigateur (mode application).");
        return 0;
    }
    fatal(&format!("La fenêtre d'Ocekour n'a pas pu s'ouvrir : {cause}\n\n{}", webview2::MISSING));
    1
}

/// Erreur fatale au démarrage : journal, sortie d'erreur et boîte de message (l'exe Windows n'a
/// pas de console : sans elle, il se fermerait sans rien dire).
pub(crate) fn fatal(text: &str) {
    log::error!("{text}");
    eprintln!("{text}");
    #[cfg(windows)]
    webview2::alert(&format!("{text}\n\nJournal : {}", ocekour_core::logging::log_file().display()));
}

/// Panique du fil principal (boucle de la fenêtre) : consignée et signalée ; avant que la page
/// soit chargée, l'interface est relancée dans le navigateur. Les paniques des tâches de fond
/// sont seulement consignées (tokio les isole).
fn install_panic_hook(args: &[String], can_fall_back: bool) {
    let args = args.to_vec();
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("Panique : {info}");
        if std::thread::current().name() != Some("main") {
            default(info);
            return;
        }
        if can_fall_back && !WINDOW_LOADED.load(Ordering::SeqCst) {
            log::error!("Fenêtre impossible (panique) : {info}");
            std::process::exit(fall_back_to_browser(&args, &info.to_string()));
        }
        fatal(&format!("Erreur inattendue : {info}"));
        default(info);
    }));
}

fn run_window(app: Arc<App>, ui: Arc<ui::TauriUi>, context: tauri::Context<tauri::Wry>) -> tauri::Result<()> {
    let core = app.clone();
    let built = tauri::Builder::default()
        // Un seul Ocekour à la fois : un second lancement remet la fenêtre au premier plan.
        // (Les modes sans interface ne passent pas par ici et peuvent tourner à côté.)
        .plugin(tauri_plugin_single_instance::init(|handle, args, cwd| {
            // « Ouvrir avec Ocekour » alors qu'il tourne déjà : fichiers ouverts dans la fenêtre.
            let paths = ocekour_core::files::paths_in_args(args.get(1..).unwrap_or_default(), std::path::Path::new(&cwd));
            if !paths.is_empty() { let _ = handle.emit("files:open", paths); }
            if let Some(window) = handle.get_webview_window(MAIN_WINDOW) {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app.clone())
        .invoke_handler(invoke_handler())
        .setup(move |tauri_app| {
            ui.attach(tauri_app.handle().clone());
            let window = create_window(tauri_app.handle(), &core)?;
            follow_theme(&core, window);
            if let Ok(watcher) = core.config.watch() { tauri_app.manage(Watcher(watcher)); }
            core.watch_mcp_http();
            core.start_background();
            Ok(())
        })
        .build(context);
    let tauri_app = built?;
    // Avant de quitter : dernier envoi des statistiques (2 s au plus), serveurs arrêtés.
    let stopping = Arc::new(AtomicBool::new(false));
    tauri_app.run(move |handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            if stopping.swap(true, Ordering::SeqCst) { return; }
            api.prevent_exit();
            let (app, handle) = (app.clone(), handle.clone());
            tauri::async_runtime::spawn(async move {
                app.shutdown().await;
                handle.exit(0);
            });
        }
    });
    Ok(())
}

/// Garde vivant l'observateur de config.yaml (modifications faites hors de l'application).
struct Watcher(#[allow(dead_code)] ocekour_core::config::Watcher);

fn create_window(handle: &tauri::AppHandle, app: &Arc<App>) -> tauri::Result<tauri::WebviewWindow> {
    let dark = app.config.config().theme == "dark";
    let streams = app.clone();
    let window = WebviewWindowBuilder::new(handle, MAIN_WINDOW, WebviewUrl::App("index.html".into()))
        .title("Ocekour")
        .inner_size(1400.0, 900.0)
        .min_inner_size(800.0, 500.0)
        .background_color(if dark { tauri::window::Color(2, 6, 23, 255) } else { tauri::window::Color(255, 255, 255, 255) })
        .on_navigation(|url| allowed_navigation(url))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        // Rechargement de la page : ses flux de logs sont arrêtés.
        .on_page_load(move |window, payload| {
            match payload.event() {
                PageLoadEvent::Started => streams.streams.close_all_of(window.label()),
                PageLoadEvent::Finished => WINDOW_LOADED.store(true, Ordering::SeqCst),
            }
        })
        .build()?;
    let streams = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event { streams.streams.close_all_of(MAIN_WINDOW); }
    });
    #[cfg(debug_assertions)]
    if std::env::var_os("OCEKOUR_DEVTOOLS").is_some() { window.open_devtools(); }
    Ok(window)
}

/// Thème natif (barre de titre) aligné sur la préférence de config.yaml.
fn follow_theme(app: &Arc<App>, window: tauri::WebviewWindow) {
    let apply = move |theme: &str| {
        let _ = window.set_theme(match theme {
            "dark" => Some(tauri::Theme::Dark),
            "light" => Some(tauri::Theme::Light),
            _ => None,
        });
    };
    apply(&app.config.config().theme);
    app.config.on_changed(move |state| apply(state.config["theme"].as_str().unwrap_or("system")));
}

/// Exe sans console (sous-système Windows) lancé depuis un terminal : la sortie (jeton du
/// serveur HTTP, version) y est rattachée. Sans terminal parent, rien.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_limitee_au_front() {
        let ok = |u: &str| allowed_navigation(&tauri::Url::parse(u).unwrap());
        assert!(ok("tauri://localhost/index.html"));
        assert!(ok("http://tauri.localhost/index.html"));
        assert!(!ok("https://exemple.fr/"));
        assert!(!ok("file:///C:/Windows/"));
        assert!(!ok("javascript:alert(1)"));
    }
}
