//! Serveur de la sonde : HTTP(S) en lecture seule, jeton unique, plusieurs postes abonnés
//! indépendants (chacun ses options et sa position), flux NDJSON avec curseur de reprise,
//! limites (total, par poste), blocage après échecs d'authentification, rechargement à chaud
//! de sonde.yaml, état des abonnés dans `sonde-statut.json`.

use super::config::{Loaded, STATUS_FILE, SourceConfig, load, token_matches};
use crate::streams::StreamEvent;
use crate::streams::file::{Cursor, FileEvent, FileOptions, FileTarget, FileTiming, spawn};
use crate::streams::pattern::{FileEntry, MAX_FILES, io_error};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::{Request, Response, StatusCode};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub const PROTOCOL: u32 = 1;
/// Échecs d'authentification tolérés par minute et par adresse, puis blocage.
const MAX_FAILURES: u32 = 10;
const BLOCK_FOR: Duration = Duration::from_secs(5 * 60);
/// Connexions TCP simultanées au plus (au-delà, refusées).
const MAX_CONNECTIONS: usize = 500;

type Body = UnsyncBoxBody<Bytes, std::convert::Infallible>;

/// Réglages de temps (réduits dans les tests).
#[derive(Debug, Clone)]
pub struct Timing {
    pub ping: Duration,
    pub reload: Duration,
    pub status: Duration,
    pub file: FileTiming,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { ping: Duration::from_secs(15), reload: Duration::from_secs(1), status: Duration::from_secs(2), file: FileTiming::default() }
    }
}

/// Un abonnement en cours (un flux ouvert par un poste).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Subscriber {
    pub poste: String,
    pub ip: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fichier: Option<String>,
    pub suivi: bool,
    /// ISO 8601.
    pub depuis: String,
    pub lignes: u64,
}

struct Sub {
    info: Subscriber,
    lines: Arc<AtomicU64>,
    cancel: CancellationToken,
    /// Raison de la fermeture par la sonde (jeton renouvelé, source retirée…).
    reason: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
struct Failures {
    count: u32,
    window: Option<Instant>,
    blocked_until: Option<Instant>,
}

struct Shared {
    config_path: PathBuf,
    loaded: RwLock<Arc<Loaded>>,
    subs: Mutex<HashMap<u64, Sub>>,
    next: AtomicU64,
    failures: Mutex<HashMap<IpAddr, Failures>>,
    timing: Timing,
    started: String,
}

/// Sonde démarrée.
pub struct Running {
    pub addr: SocketAddr,
    stop: CancellationToken,
    shared: Arc<Shared>,
}

impl Running {
    pub fn stop(&self) {
        self.stop.cancel();
        for (_, sub) in self.shared.subs.lock().drain() { sub.cancel.cancel(); }
    }
    pub fn subscribers(&self) -> Vec<Subscriber> { self.shared.subscribers() }
    pub async fn stopped(&self) { self.stop.cancelled().await }
}

impl Drop for Running {
    fn drop(&mut self) { self.stop(); }
}

impl Shared {
    fn config(&self) -> Arc<Loaded> { self.loaded.read().clone() }

    fn subscribers(&self) -> Vec<Subscriber> {
        let mut list: Vec<Subscriber> = self.subs.lock().values().map(|s| Subscriber { lignes: s.lines.load(Ordering::Relaxed), ..s.info.clone() }).collect();
        list.sort_by(|a, b| a.depuis.cmp(&b.depuis).then_with(|| a.poste.cmp(&b.poste)));
        list
    }

    /// Ferme des abonnements (la sonde envoie la raison avant de couper).
    fn close_where(&self, reason: &str, filter: impl Fn(&Sub) -> bool) {
        let subs = self.subs.lock();
        for sub in subs.values().filter(|s| filter(s)) {
            *sub.reason.lock() = Some(reason.to_string());
            sub.cancel.cancel();
        }
    }

    fn write_status(&self) {
        let loaded = self.config();
        let status = json!({
            "nom": loaded.name(), "version": env!("CARGO_PKG_VERSION"), "ecoute": loaded.config.ecoute,
            "demarre": self.started, "misAJour": crate::text::now_iso(), "pid": std::process::id(),
            "abonnes": self.subscribers(),
        });
        let path = loaded.dir.join(STATUS_FILE);
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(&status).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn reload(&self) {
        let next = match load(&self.config_path) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("Rechargement ignoré : {e}");
                return;
            }
        };
        let current = self.config();
        if *current == next { return; }
        if next.addr != current.addr || next.tls != current.tls {
            log::warn!("Adresse d'écoute ou certificat modifiés : redémarrez la sonde pour les appliquer.");
        }
        if next.config.jeton != current.config.jeton {
            log::info!("Jeton renouvelé : tous les abonnements sont fermés.");
            self.close_where("Jeton de la sonde renouvelé : mettez à jour la connexion.", |_| true);
        }
        for old in &current.config.sources {
            match next.source(&old.id) {
                None => self.close_where("Source retirée de la sonde.", |s| s.info.source == old.id),
                Some(new) if new != old => self.close_where("Source modifiée sur la sonde : rouvrez l'onglet.", |s| s.info.source == old.id),
                _ => {}
            }
        }
        log::info!("sonde.yaml rechargé ({} source(s)).", next.config.sources.len());
        *self.loaded.write() = Arc::new(next);
    }
}

/// Démarre la sonde décrite par `config_path`.
pub async fn start(config_path: &Path, timing: Timing) -> anyhow::Result<Running> {
    crate::net::install_default_provider();
    let loaded = load(config_path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let tls = match &loaded.tls {
        Some((cert, key)) => Some(tls_acceptor(cert, key)?),
        None => None,
    };
    let listener = TcpListener::bind(loaded.addr).await.map_err(|e| anyhow::anyhow!("Écoute sur {} impossible : {e}", loaded.addr))?;
    let addr = listener.local_addr()?;
    let stop = CancellationToken::new();
    let shared = Arc::new(Shared {
        config_path: config_path.to_path_buf(),
        loaded: RwLock::new(Arc::new(loaded)),
        subs: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
        failures: Mutex::new(HashMap::new()),
        timing,
        started: crate::text::now_iso(),
    });
    log::info!("Sonde « {} » : {}://{addr} ({} source(s))", shared.config().name(), if tls.is_some() { "https" } else { "http" }, shared.config().config.sources.len());
    shared.write_status();

    // Rechargement de sonde.yaml et état des abonnés.
    {
        let (shared, stop) = (shared.clone(), stop.clone());
        tokio::spawn(async move {
            let mut reload = tokio::time::interval(shared.timing.reload);
            let mut status = tokio::time::interval(shared.timing.status);
            let mut last = vec![];
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = reload.tick() => {
                        let s = shared.clone();
                        let _ = tokio::task::spawn_blocking(move || s.reload()).await;
                    }
                    _ = status.tick() => {
                        let now = shared.subscribers();
                        if now != last { shared.write_status(); last = now; }
                    }
                }
            }
            shared.write_status();
        });
    }

    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    {
        let (shared, stop) = (shared.clone(), stop.clone());
        tokio::spawn(async move {
            loop {
                let (stream, peer) = tokio::select! {
                    _ = stop.cancelled() => break,
                    accepted = listener.accept() => match accepted { Ok(s) => s, Err(_) => continue },
                };
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
                let (shared, stop, tls) = (shared.clone(), stop.clone(), tls.clone());
                tokio::spawn(async move {
                    let _permit = permit;
                    let service = hyper::service::service_fn(move |req| {
                        let shared = shared.clone();
                        async move { Ok::<_, std::convert::Infallible>(handle(shared, peer.ip(), req).await) }
                    });
                    let builder = {
                        let mut b = hyper::server::conn::http1::Builder::new();
                        b.timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(Duration::from_secs(10));
                        b
                    };
                    match tls {
                        Some(acceptor) => {
                            let Ok(Ok(stream)) = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await else { return };
                            let conn = builder.serve_connection(hyper_util::rt::TokioIo::new(stream), service);
                            tokio::pin!(conn);
                            tokio::select! { _ = conn.as_mut() => {}, _ = stop.cancelled() => conn.as_mut().graceful_shutdown() }
                        }
                        None => {
                            let conn = builder.serve_connection(hyper_util::rt::TokioIo::new(stream), service);
                            tokio::pin!(conn);
                            tokio::select! { _ = conn.as_mut() => {}, _ = stop.cancelled() => conn.as_mut().graceful_shutdown() }
                        }
                    }
                });
            }
        });
    }
    Ok(Running { addr, stop, shared })
}

fn tls_acceptor(cert: &Path, key: &Path) -> anyhow::Result<tokio_rustls::TlsAcceptor> {
    let read = |p: &Path| std::fs::read(p).map_err(|e| anyhow::anyhow!("{} : {e}", p.display()));
    let certs: Vec<_> = rustls_pemfile::certs(&mut read(cert)?.as_slice()).collect::<Result<_, _>>()?;
    if certs.is_empty() { anyhow::bail!("{} : aucun certificat PEM", cert.display()); }
    let key = rustls_pemfile::private_key(&mut read(key)?.as_slice())?.ok_or_else(|| anyhow::anyhow!("{} : aucune clé privée PEM", key.display()))?;
    let config = rustls::ServerConfig::builder_with_provider(crate::net::provider())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

// --- Requêtes ----------------------------------------------------------------------------------

fn full(status: StatusCode, body: Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Full::new(Bytes::from(serde_json::to_vec(&body).unwrap_or_default())).boxed_unsync())
        .unwrap()
}

fn error(status: StatusCode, message: impl Into<String>) -> Response<Body> { full(status, json!({ "erreur": message.into() })) }

fn query(req: &Request<Incoming>) -> HashMap<String, String> {
    req.uri().query().map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()).unwrap_or_default()
}

fn header<'a>(req: &'a Request<Incoming>, name: &str) -> Option<&'a str> { req.headers().get(name).and_then(|h| h.to_str().ok()) }

/// Authentification ; `Err` = réponse de refus.
fn authenticate(shared: &Shared, ip: IpAddr, req: &Request<Incoming>) -> Result<(), Response<Body>> {
    let now = Instant::now();
    {
        let mut failures = shared.failures.lock();
        let f = failures.entry(ip).or_default();
        if let Some(until) = f.blocked_until {
            if until > now {
                let minutes = (until - now).as_secs().div_ceil(60);
                return Err(error(StatusCode::TOO_MANY_REQUESTS, format!("Trop d'échecs d'authentification : réessayez dans {minutes} min.")));
            }
            *f = Failures::default();
        }
    }
    let given = header(req, "authorization").and_then(|a| a.strip_prefix("Bearer ")).unwrap_or_default();
    if !given.is_empty() && token_matches(given, &shared.config().config.jeton) { return Ok(()); }
    let mut failures = shared.failures.lock();
    let f = failures.entry(ip).or_default();
    if f.window.is_none_or(|w| now - w > Duration::from_secs(60)) { f.window = Some(now); f.count = 0; }
    f.count += 1;
    if f.count >= MAX_FAILURES {
        f.blocked_until = Some(now + BLOCK_FOR);
        log::warn!("Authentification : {ip} bloquée {} min après {MAX_FAILURES} échecs", BLOCK_FOR.as_secs() / 60);
    } else {
        log::warn!("Authentification refusée : {ip}");
    }
    Err(error(StatusCode::UNAUTHORIZED, "Jeton de la sonde absent ou invalide."))
}

async fn handle(shared: Arc<Shared>, ip: IpAddr, req: Request<Incoming>) -> Response<Body> {
    if let Err(refused) = authenticate(&shared, ip, &req) { return refused; }
    if req.method() != hyper::Method::GET { return error(StatusCode::METHOD_NOT_ALLOWED, "Méthode non prise en charge (GET)."); }
    let poste = header(&req, "x-ocekour-poste").map(|p| p.trim().chars().filter(|c| !c.is_control()).take(100).collect::<String>()).filter(|p| !p.is_empty()).unwrap_or_else(|| ip.to_string());
    let path = req.uri().path().trim_end_matches('/').to_string();
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    let loaded = shared.config();
    match segments.as_slice() {
        ["v1", "info"] => full(StatusCode::OK, json!({
            "nom": loaded.name(), "version": env!("CARGO_PKG_VERSION"), "protocole": PROTOCOL,
            "os": std::env::consts::OS, "sources": loaded.config.sources.len(),
        })),
        ["v1", "sources"] => {
            let sources = loaded.config.sources.clone();
            let list = tokio::task::spawn_blocking(move || sources.iter().map(describe).collect::<Vec<_>>()).await.unwrap_or_default();
            full(StatusCode::OK, json!(list))
        }
        ["v1", "sources", id, "fichiers"] => {
            let Some(source) = loaded.source(id).cloned() else { return error(StatusCode::NOT_FOUND, format!("Source « {id} » inconnue.")) };
            match tokio::task::spawn_blocking(move || files_of(&source)).await {
                Ok(Ok(files)) => full(StatusCode::OK, json!(files)),
                Ok(Err((status, message))) => error(status, message),
                Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Erreur interne."),
            }
        }
        ["v1", "sources", id, "flux"] => {
            let Some(source) = loaded.source(id).cloned() else { return error(StatusCode::NOT_FOUND, format!("Source « {id} » inconnue.")) };
            let gzip = header(&req, "accept-encoding").is_some_and(|a| a.contains("gzip"));
            stream(shared.clone(), &loaded, source, query(&req), poste, ip, gzip)
        }
        _ => error(StatusCode::NOT_FOUND, "Chemins : /v1/info, /v1/sources, /v1/sources/{id}/fichiers, /v1/sources/{id}/flux"),
    }
}

/// État d'une source (liste des sources, `sonde verifier`).
pub fn describe(s: &SourceConfig) -> Value {
    let (etat, newest, count) = match s.target() {
        FileTarget::File(p) => match FileEntry::of(&p) {
            Ok(f) => (readable(&p), Some(f), None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ("absent", None, None),
            Err(_) => ("illisible", None, None),
        },
        FileTarget::Pattern(p) => match p.list(s.garder.unwrap_or(MAX_FILES)) {
            Ok(files) => {
                let first = files.first().cloned();
                (if first.is_some() { first.as_ref().map(|f| readable(&f.path)).unwrap_or("ok") } else { "absent" }, first, Some(files.len()))
            }
            Err(e) if e.code == crate::error::ErrorCode::NotFound => ("absent", None, Some(0)),
            Err(_) => ("illisible", None, None),
        },
    };
    json!({
        "id": s.id, "nom": s.name(), "type": if s.is_pattern() { "motif" } else { "fichier" },
        "etat": etat, "taille": newest.as_ref().map(|f| f.size), "modifie": newest.and_then(|f| f.modified),
        "fichiers": count,
    })
}

fn readable(path: &Path) -> &'static str { if crate::streams::file::open_shared(path).is_ok() { "ok" } else { "illisible" } }

fn files_of(source: &SourceConfig) -> Result<Vec<FileEntry>, (StatusCode, String)> {
    let status = |e: crate::error::SourceError| {
        let code = match e.code {
            crate::error::ErrorCode::NotFound => StatusCode::NOT_FOUND,
            crate::error::ErrorCode::Forbidden => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_REQUEST,
        };
        (code, e.message)
    };
    match source.target() {
        FileTarget::File(p) => FileEntry::of(&p).map(|f| vec![f]).map_err(|e| status(io_error(&p, &e))),
        FileTarget::Pattern(p) => p.list(source.garder.unwrap_or(MAX_FILES)).map_err(status),
    }
}

/// Retire l'abonnement quand le flux se termine (poste parti compris).
struct Guard {
    shared: Arc<Shared>,
    id: u64,
    cancel: CancellationToken,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(sub) = self.shared.subs.lock().remove(&self.id) {
            log::info!("Abonnement terminé : poste {}, source {}, {} ligne(s)", sub.info.poste, sub.info.source, sub.lines.load(Ordering::Relaxed));
        }
    }
}

/// Encodeur du corps : NDJSON, compressé en gzip si le poste l'accepte (vidé à chaque événement).
struct Encoder(Option<flate2::write::GzEncoder<Vec<u8>>>);

impl Encoder {
    fn encode(&mut self, value: &Value) -> Bytes {
        let mut line = serde_json::to_vec(value).unwrap_or_default();
        line.push(b'\n');
        match &mut self.0 {
            None => Bytes::from(line),
            Some(gz) => {
                let _ = gz.write_all(&line);
                let _ = gz.flush();
                Bytes::from(std::mem::take(gz.get_mut()))
            }
        }
    }
}

fn event_json(e: &FileEvent) -> Value {
    let mut v = serde_json::to_value(&e.event).unwrap_or(Value::Null);
    if let (StreamEvent::Lines { .. }, Some(c)) = (&e.event, &e.cursor) { v["curseur"] = json!(c.encode()); }
    v
}

fn stream(shared: Arc<Shared>, loaded: &Loaded, source: SourceConfig, q: HashMap<String, String>, poste: String, ip: IpAddr, gzip: bool) -> Response<Body> {
    let limits = &loaded.config.limites;
    {
        let subs = shared.subs.lock();
        if subs.len() >= limits.flux_simultanes {
            log::warn!("Limite atteinte ({} flux) : poste {poste} refusé", limits.flux_simultanes);
            return error(StatusCode::TOO_MANY_REQUESTS, format!("Sonde saturée : {} flux ouverts au plus.", limits.flux_simultanes));
        }
        if subs.values().filter(|s| s.info.poste == poste).count() >= limits.flux_par_poste {
            return error(StatusCode::TOO_MANY_REQUESTS, format!("Trop de flux ouverts depuis ce poste sur cette sonde ({} au plus) : fermez des onglets.", limits.flux_par_poste));
        }
    }
    let number = |k: &str| q.get(k).and_then(|v| v.parse::<u64>().ok());
    let fichier = q.get("fichier").filter(|f| !f.is_empty()).cloned();
    let target = match (source.target(), &fichier) {
        (FileTarget::Pattern(p), Some(name)) => match p.file(name) {
            Ok(f) => FileTarget::File(f),
            Err(e) => return error(StatusCode::NOT_FOUND, e.message),
        },
        (FileTarget::File(_), Some(_)) => return error(StatusCode::BAD_REQUEST, "« fichier » : seulement pour une source de type motif."),
        (t, None) => t,
    };
    let resume = match q.get("reprise").filter(|r| !r.is_empty()) {
        Some(r) => match Cursor::decode(r) {
            Some(c) => Some(c),
            None => return error(StatusCode::BAD_REQUEST, "Curseur de reprise invalide."),
        },
        None => None,
    };
    let follow = q.get("suivi").is_some_and(|v| v == "1" || v == "true");
    let options = FileOptions { follow, tail_lines: number("lignes"), since_seconds: number("depuis").filter(|s| *s > 0), encoding: source.encoding(), resume };

    let id = shared.next.fetch_add(1, Ordering::Relaxed);
    let cancel = CancellationToken::new();
    let lines = Arc::new(AtomicU64::new(0));
    let reason = Arc::new(Mutex::new(None));
    let info = Subscriber { poste: poste.clone(), ip: ip.to_string(), source: source.id.clone(), fichier: fichier.clone(), suivi: follow, depuis: crate::text::now_iso(), lignes: 0 };
    log::info!("Abonnement : poste {poste} ({ip}), source {}{}{}", source.id, fichier.map(|f| format!(", fichier {f}")).unwrap_or_default(), if follow { ", suivi" } else { "" });
    shared.subs.lock().insert(id, Sub { info, lines: lines.clone(), cancel: cancel.clone(), reason: reason.clone() });

    let (tx, rx) = tokio::sync::mpsc::channel(32);
    spawn(target, options, shared.timing.file.clone(), cancel.clone(), tx);
    let guard = Guard { shared: shared.clone(), id, cancel: cancel.clone() };
    let ping = shared.timing.ping;
    let encoder = Encoder(gzip.then(|| flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast())));
    struct State {
        rx: tokio::sync::mpsc::Receiver<FileEvent>,
        guard: Guard,
        encoder: Encoder,
        lines: Arc<AtomicU64>,
        reason: Arc<Mutex<Option<String>>>,
        done: bool,
    }
    let state = State { rx, guard, encoder, lines, reason, done: false };
    // `ping` après un moment sans rien envoyer : le poste détecte une connexion morte.
    let body = futures_util::stream::unfold(state, move |mut s| {
        async move {
            if s.done { return None; }
            let value = tokio::select! {
                biased;
                _ = s.guard.cancel.cancelled() => {
                    s.done = true;
                    // Fermé par la sonde (jeton, source) : le poste est prévenu. Sonde arrêtée :
                    // la connexion est simplement coupée, le poste se reconnectera et reprendra.
                    let message = s.reason.lock().clone()?;
                    json!({ "type": "status", "state": "ended", "message": message })
                }
                e = s.rx.recv() => match e {
                    Some(e) => {
                        if let StreamEvent::Lines { lines } = &e.event { s.lines.fetch_add(lines.len() as u64, Ordering::Relaxed); }
                        event_json(&e)
                    }
                    None => return None,
                },
                _ = tokio::time::sleep(ping) => json!({ "type": "ping" }),
            };
            let bytes = s.encoder.encode(&value);
            Some((Ok::<_, std::convert::Infallible>(Frame::data(bytes)), s))
        }
    });
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/x-ndjson; charset=utf-8")
        .header("cache-control", "no-store")
        .header("x-accel-buffering", "no");
    if gzip { builder = builder.header("content-encoding", "gzip"); }
    builder.body(BodyExt::boxed_unsync(StreamBody::new(body.boxed()))).unwrap()
}

/// État écrit par une sonde en marche (`sonde statut`).
pub fn read_status(dir: &Path) -> anyhow::Result<Value> {
    let path = dir.join(STATUS_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("{} : {e} (la sonde est-elle démarrée ?)", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}
