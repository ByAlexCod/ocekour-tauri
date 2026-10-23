//! Mode Edge : quand WebView2 n'est pas utilisable (Windows 10 hors ligne, sans droits
//! d'installation), l'interface s'affiche dans le navigateur Edge du poste en mode application
//! (`msedge --app` : fenêtre dédiée, sans onglets ni barre d'adresse). Un serveur HTTP local
//! remplace la commande `call` et les événements de Tauri :
//!
//! - `127.0.0.1` uniquement, port libre, hôte vérifié (DNS rebinding) ;
//! - jeton aléatoire : `/?t=<jeton>` (URL donnée à Edge) le dépose dans un cookie `HttpOnly`,
//!   `SameSite=Strict`, exigé ensuite sur chaque requête ;
//! - `POST /api/call` : mêmes canaux que la fenêtre Tauri ;
//! - `GET /api/events` : événements du cœur (Server-Sent Events) ;
//! - le reste : l'interface, servie depuis les fichiers déjà intégrés à l'exe par Tauri.
//!
//! Ocekour s'arrête quand la fenêtre Edge est fermée (plus aucun abonné aux événements).

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use ocekour_core::api::{self, Caller};
use ocekour_core::app::App;
use ocekour_core::ui::{FileFilter, OpenOptions, Ui};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;

type Body = UnsyncBoxBody<Bytes, Infallible>;

/// Fichiers de l'interface intégrés à l'exe (ceux de la fenêtre Tauri).
pub type Assets = &'static dyn tauri::Assets<tauri::Wry>;

const COOKIE: &str = "ocekour";
/// Requête au plus (export de lignes : contenu envoyé par l'interface).
const MAX_BODY: usize = 256 * 1024 * 1024;
/// Signe de vie vers Edge : une fenêtre fermée est détectée au plus tard après ce délai.
const HEARTBEAT: Duration = Duration::from_secs(5);
/// Fenêtre fermée (aucun abonné) depuis ce délai : Ocekour s'arrête. Un rechargement de la page
/// se reconnecte bien avant.
const IDLE_EXIT: Duration = Duration::from_secs(10);
/// Edge ne s'est jamais connecté (lancement impossible, fenêtre fermée aussitôt) : arrêt.
const FIRST_CONNECT: Duration = Duration::from_secs(120);

/// Variable : navigateur à lancer à la place d'Edge (tests, autre navigateur Chromium).
pub const BROWSER_VAR: &str = "OCEKOUR_APP_BROWSER";

/// `msedge.exe` d'Edge installé : `<base>\Microsoft\Edge\Application\msedge.exe` (Program Files
/// (x86), Program Files, puis installation par utilisateur).
pub fn edge_in(bases: &[PathBuf]) -> Option<PathBuf> {
    bases.iter().map(|b| b.join("Microsoft").join("Edge").join("Application").join("msedge.exe")).find(|p| p.is_file())
}

/// Navigateur du mode Edge : la variable `OCEKOUR_APP_BROWSER`, sinon Edge (Windows).
pub fn find_browser() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(BROWSER_VAR).filter(|p| !p.is_empty()) { return Some(path.into()); }
    let bases: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    edge_in(&bases)
}

/// Ce que le mode Edge apporte au cœur : événements vers la page, boîtes de dialogue natives,
/// presse-papiers, notifications système (sans WebView2).
pub struct BrowserUi {
    events: broadcast::Sender<Bytes>,
}

impl Default for BrowserUi {
    fn default() -> Self { BrowserUi { events: broadcast::channel(4096).0 } }
}

/// Un événement au format Server-Sent Events.
fn sse(event: &str, payload: &Value) -> Bytes {
    Bytes::from(format!("event: {event}\ndata: {}\n\n", serde_json::to_string(payload).unwrap_or_else(|_| "null".into())))
}

fn dialog(title: &str, default_path: Option<PathBuf>, filters: &[FileFilter]) -> rfd::AsyncFileDialog {
    let mut d = rfd::AsyncFileDialog::new().set_title(title);
    if let Some(path) = default_path {
        match (path.extension().is_some() || !path.is_dir(), path.parent(), path.file_name()) {
            (true, Some(dir), Some(name)) => d = d.set_directory(dir).set_file_name(name.to_string_lossy()),
            _ => d = d.set_directory(&path),
        }
    }
    for f in filters {
        let extensions: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        d = d.add_filter(&f.name, &extensions);
    }
    d
}

#[async_trait::async_trait]
impl Ui for BrowserUi {
    fn emit(&self, event: &str, payload: Value) {
        // Aucun abonné (fenêtre en cours d'ouverture) : l'événement est perdu, comme avec Tauri.
        let _ = self.events.send(sse(event, &payload));
    }

    async fn save_dialog(&self, title: &str, default_path: PathBuf, filters: Vec<FileFilter>) -> Option<PathBuf> {
        dialog(title, Some(default_path), &filters).save_file().await.map(|h| h.path().to_path_buf())
    }

    async fn open_dialog(&self, options: OpenOptions) -> Vec<PathBuf> {
        let d = dialog(&options.title, options.default_path.clone(), &options.filters);
        let handles = match (options.directory, options.multiple) {
            (true, true) => d.pick_folders().await.unwrap_or_default(),
            (true, false) => d.pick_folder().await.into_iter().collect(),
            (false, true) => d.pick_files().await.unwrap_or_default(),
            (false, false) => d.pick_file().await.into_iter().collect(),
        };
        handles.iter().map(|h| h.path().to_path_buf()).collect()
    }

    fn copy_text(&self, text: &str) -> anyhow::Result<()> {
        arboard::Clipboard::new()?.set_text(text.to_string()).map_err(|e| anyhow::anyhow!("Presse-papiers : {e}"))
    }

    fn read_clipboard(&self) -> anyhow::Result<String> {
        arboard::Clipboard::new()?.get_text().map_err(|e| anyhow::anyhow!("Presse-papiers : {e}"))
    }

    fn notify(&self, title: &str, body: &str) {
        if let Err(e) = notify_rust::Notification::new().summary(title).body(body).show() {
            log::warn!("Notification système impossible : {e}");
        }
    }
}

struct Server {
    app: Arc<App>,
    ui: Arc<BrowserUi>,
    assets: Assets,
    token: String,
    port: u16,
    /// Abonnés aux événements (fenêtres Edge ouvertes).
    clients: AtomicUsize,
    /// Une fenêtre s'est déjà connectée.
    connected_once: AtomicBool,
    /// Depuis quand aucune fenêtre n'est connectée.
    idle_since: Mutex<Instant>,
}

/// Démarre le serveur et la fenêtre Edge ; rend la main quand la fenêtre est fermée.
pub async fn run(app: Arc<App>, ui: Arc<BrowserUi>, assets: Assets, browser: PathBuf, profile: PathBuf) -> i32 {
    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(l) => l,
        Err(e) => {
            crate::fatal(&format!("Mode Edge : écoute impossible : {e}"));
            return 1;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let server = Arc::new(Server {
        app: app.clone(),
        ui,
        assets,
        token: ocekour_core::sonde::config::new_token(),
        port,
        clients: AtomicUsize::new(0),
        connected_once: AtomicBool::new(false),
        idle_since: Mutex::new(Instant::now()),
    });
    // Comme la fenêtre Tauri au démarrage.
    let _watcher = app.config.watch().ok();
    app.watch_mcp_http();
    app.start_background();

    let url = format!("http://127.0.0.1:{port}/?t={}", server.token);
    log::info!("Mode Edge (WebView2 indisponible) : interface sur http://127.0.0.1:{port}/ avec {}", browser.display());
    if let Err(e) = launch(&browser, &url, &profile) {
        crate::fatal(&format!("Ocekour n'a pas pu ouvrir sa fenêtre avec Edge ({}) : {e}", browser.display()));
        app.shutdown().await;
        return 1;
    }

    let started = Instant::now();
    let mut check = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let server = server.clone();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        let server = server.clone();
                        async move { Ok::<_, Infallible>(handle(server, req).await) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
                });
            }
            _ = check.tick() => {
                if server.clients.load(Ordering::SeqCst) > 0 { continue; }
                let closed = server.connected_once.load(Ordering::SeqCst) && server.idle_since.lock().elapsed() >= IDLE_EXIT;
                let never = !server.connected_once.load(Ordering::SeqCst) && started.elapsed() >= FIRST_CONNECT;
                if closed {
                    log::info!("Mode Edge : fenêtre fermée, arrêt.");
                    break;
                }
                if never {
                    crate::fatal(&format!("La fenêtre Edge ({}) ne s'est pas connectée à Ocekour en {} s : arrêt.", browser.display(), FIRST_CONNECT.as_secs()));
                    app.shutdown().await;
                    return 1;
                }
            }
        }
    }
    app.shutdown().await;
    0
}

/// Edge en mode application, avec un profil propre à Ocekour (fenêtre indépendante du
/// navigateur de l'utilisateur, aucune extension ni onglet restauré).
fn launch(browser: &Path, url: &str, profile: &Path) -> std::io::Result<()> {
    std::process::Command::new(browser)
        .arg(format!("--app={url}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(["--no-first-run", "--no-default-browser-check", "--window-size=1400,900"])
        .spawn()
        .map(|_| ())
}

fn same_token(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn full(status: StatusCode, content_type: &str, body: impl Into<Bytes>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        .body(Full::new(body.into()).boxed_unsync())
        .unwrap()
}

fn text(status: StatusCode, message: &str) -> Response<Body> { full(status, "text/plain; charset=utf-8", message.to_string()) }

fn json_reply(status: StatusCode, value: &Value) -> Response<Body> {
    full(status, "application/json", serde_json::to_vec(value).unwrap_or_default())
}

fn cookie_token(req: &Request<Incoming>) -> Option<String> {
    req.headers()
        .get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(&format!("{COOKIE}=")).map(str::to_string))
}

async fn handle(s: Arc<Server>, req: Request<Incoming>) -> Response<Body> {
    // Seul 127.0.0.1:<port> est servi (une page d'un autre site ne peut pas se faire passer pour lui).
    let host = req.headers().get("host").and_then(|h| h.to_str().ok()).unwrap_or("");
    if host != format!("127.0.0.1:{}", s.port) { return text(StatusCode::FORBIDDEN, "Hôte refusé."); }
    let path = req.uri().path().to_string();

    // Entrée : l'URL donnée à Edge porte le jeton, déposé dans un cookie puis retiré de l'adresse.
    if req.method() == Method::GET && path == "/" {
        if let Some(given) = req.uri().query().and_then(|q| q.split('&').find_map(|p| p.strip_prefix("t="))) {
            if !same_token(given, &s.token) { return text(StatusCode::FORBIDDEN, "Lien périmé : relancez Ocekour."); }
            return Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header("location", "/")
                .header("set-cookie", format!("{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/", s.token))
                .header("cache-control", "no-store")
                .body(Full::new(Bytes::new()).boxed_unsync())
                .unwrap();
        }
    }
    if !cookie_token(&req).is_some_and(|t| same_token(&t, &s.token)) {
        return text(StatusCode::FORBIDDEN, "Session expirée : relancez Ocekour.");
    }

    match (req.method().clone(), path.as_str()) {
        (Method::POST, "/api/call") => call(s, req).await,
        (Method::GET, "/api/events") => events(s),
        (Method::GET, _) => asset(&s, &path),
        _ => text(StatusCode::NOT_FOUND, "Introuvable."),
    }
}

async fn call(s: Arc<Server>, req: Request<Incoming>) -> Response<Body> {
    // Requête d'une autre origine (formulaire d'un site) : refusée, même avec le cookie.
    if let Some(origin) = req.headers().get("origin").and_then(|o| o.to_str().ok()) {
        if origin != format!("http://127.0.0.1:{}", s.port) { return text(StatusCode::FORBIDDEN, "Origine refusée."); }
    }
    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return text(StatusCode::PAYLOAD_TOO_LARGE, "Requête trop volumineuse."),
    };
    let Ok(request) = serde_json::from_slice::<Value>(&body) else { return text(StatusCode::BAD_REQUEST, "JSON invalide.") };
    let channel = request["channel"].as_str().unwrap_or_default().to_string();
    let args = request["args"].as_array().cloned().unwrap_or_default();
    match api::call(&s.app, &Caller { window: crate::MAIN_WINDOW.into() }, &channel, args).await {
        Ok(value) => json_reply(StatusCode::OK, &value),
        Err(error) => json_reply(StatusCode::INTERNAL_SERVER_ERROR, &json!({ "error": error })),
    }
}

fn events(s: Arc<Server>) -> Response<Body> {
    let mut rx = s.ui.events.subscribe();
    // Page (re)chargée : les flux ouverts par la page précédente sont fermés (comme Tauri).
    s.app.streams.close_all_of(crate::MAIN_WINDOW);
    s.clients.fetch_add(1, Ordering::SeqCst);
    s.connected_once.store(true, Ordering::SeqCst);
    let (tx, out) = mpsc::channel::<Bytes>(256);
    tokio::spawn(async move {
        let _ = tx.send(Bytes::from_static(b"retry: 1000\n: ok\n\n")).await;
        let mut beat = tokio::time::interval(HEARTBEAT);
        loop {
            tokio::select! {
                event = rx.recv() => match event {
                    Ok(bytes) => if tx.send(bytes).await.is_err() { break },
                    Err(broadcast::error::RecvError::Lagged(n)) => log::warn!("Mode Edge : {n} événement(s) perdu(s) (fenêtre trop lente)."),
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                // Fenêtre fermée : l'envoi échoue (connexion coupée).
                _ = beat.tick() => if tx.send(Bytes::from_static(b": ping\n\n")).await.is_err() { break },
            }
        }
        *s.idle_since.lock() = Instant::now();
        s.clients.fetch_sub(1, Ordering::SeqCst);
    });
    let body = StreamBody::new(tokio_stream::wrappers::ReceiverStream::new(out).map(|b| Ok::<_, Infallible>(Frame::data(b))));
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-store")
        .body(BodyExt::boxed_unsync(body))
        .unwrap()
}

/// Même politique que la fenêtre Tauri : seul le front d'Ocekour s'exécute.
const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'";

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn asset(s: &Server, path: &str) -> Response<Body> {
    let key = if path == "/" { "/index.html" } else { path };
    let Some(bytes) = s.assets.get(&key.into()) else { return text(StatusCode::NOT_FOUND, "Introuvable.") };
    let mut response = full(StatusCode::OK, content_type(key), bytes.into_owned());
    if key.ends_with(".html") { response.headers_mut().insert("content-security-policy", CSP.parse().unwrap()); }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_installe() {
        let (x86, local) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_eq!(edge_in(&[x86.path().into(), local.path().into()]), None);
        let exe = local.path().join("Microsoft").join("Edge").join("Application");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::write(exe.join("msedge.exe"), "").unwrap();
        assert_eq!(edge_in(&[x86.path().into(), local.path().into()]), Some(exe.join("msedge.exe")));
    }

    #[test]
    fn evenement_sse() {
        assert_eq!(&sse("files:open", &json!(["a b"]))[..], b"event: files:open\ndata: [\"a b\"]\n\n");
        assert!(same_token("abc", "abc") && !same_token("abd", "abc") && !same_token("ab", "abc"));
        assert_eq!(content_type("/assets/index-x.js"), "text/javascript; charset=utf-8");
    }
}
