//! Côté poste : connexion de type « sonde ». C'est le poste qui se connecte à la sonde
//! (HTTPS, jeton, certificat épinglé par son empreinte) ; flux NDJSON avec reconnexion et
//! reprise par curseur (ni perte ni doublon).

use crate::config::SondeConnection;
use crate::error::{ErrorCode, SourceError};
use crate::rancher::SourceNode;
use crate::streams::lines::LineSplitter;
use crate::streams::{Emit, StreamEvent, StreamOptions, StreamState, Timing};
use futures_util::StreamExt;
use serde_json::Value;
use std::io::Write;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Sans rien recevoir (ni ligne ni `ping`) pendant ce délai, la connexion est jugée morte.
pub const DEAD_AFTER: Duration = Duration::from_secs(45);

/// Identifiant du poste envoyé à la sonde (journal, limites par poste ; pas une authentification).
pub fn poste() -> String {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    format!("{}/{}", super::config::hostname(), user)
}

fn base(c: &SondeConnection) -> String { c.url.trim_end_matches('/').to_string() }

fn request(c: &SondeConnection, token: &str, path: &str) -> Result<reqwest::RequestBuilder, SourceError> {
    let client = crate::net::http_client(&c.tls)?;
    Ok(client
        .get(format!("{}{path}", base(c)))
        .bearer_auth(token)
        .header("x-ocekour-poste", poste())
        .header("user-agent", concat!("Ocekour/", env!("CARGO_PKG_VERSION"))))
}

async fn refused(response: reqwest::Response) -> SourceError {
    let status = response.status();
    let message = response.json::<Value>().await.ok().and_then(|v| v["erreur"].as_str().map(str::to_string)).unwrap_or_else(|| format!("HTTP {status}"));
    let code = match status.as_u16() {
        401 => ErrorCode::Unauthorized,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        _ => ErrorCode::Http,
    };
    SourceError::new(code, message)
}

async fn get_json(c: &SondeConnection, token: &str, path: &str) -> Result<Value, SourceError> {
    let response = request(c, token, path)?.timeout(Duration::from_secs(20)).send().await.map_err(|e| crate::net::from_reqwest(&e, &c.url))?;
    if !response.status().is_success() { return Err(refused(response).await); }
    response.json().await.map_err(|e| SourceError::invalid(format!("{} : réponse inattendue ({e}) — est-ce bien une sonde Ocekour ?", c.url)))
}

/// « Tester » : nom, version et nombre de sources.
pub async fn test(c: &SondeConnection, token: &str) -> Result<String, SourceError> {
    let info = get_json(c, token, "/v1/info").await?;
    if info["protocole"].as_u64() != Some(super::server::PROTOCOL as u64) {
        return Err(SourceError::invalid(format!("Protocole de sonde non pris en charge ({}) : mettez la sonde ou Ocekour à jour.", info["protocole"])));
    }
    Ok(format!("Sonde « {} » ({}, version {}) · {} source(s)", info["nom"].as_str().unwrap_or("?"), info["os"].as_str().unwrap_or("?"), info["version"].as_str().unwrap_or("?"), info["sources"]))
}

/// Arbre : [] → sources ; [source motif] → ses fichiers.
pub async fn browse(c: &SondeConnection, token: &str, path: &[String]) -> Result<Vec<SourceNode>, SourceError> {
    match path {
        [] => {
            let sources = get_json(c, token, "/v1/sources").await?;
            Ok(sources.as_array().cloned().unwrap_or_default().iter().map(|s| {
                let pattern = s["type"] == "motif";
                SourceNode {
                    id: s["id"].as_str().unwrap_or_default().to_string(),
                    kind: if pattern { "pattern" } else { "source" },
                    expandable: pattern,
                    health: Some(match s["etat"].as_str() { Some("ok") => "ok", Some("illisible") => "error", _ => "off" }),
                    state: s["nom"].as_str().map(str::to_string),
                    size: s["taille"].as_u64(),
                    modified: s["modifie"].as_str().map(str::to_string),
                    ..Default::default()
                }
            }).collect())
        }
        [source] => {
            let id = percent_encoding::utf8_percent_encode(source, percent_encoding::NON_ALPHANUMERIC);
            let files = get_json(c, token, &format!("/v1/sources/{id}/fichiers")).await?;
            Ok(files.as_array().cloned().unwrap_or_default().iter().map(|f| SourceNode {
                id: f["name"].as_str().unwrap_or_default().to_string(),
                kind: "file",
                expandable: false,
                health: Some("ok"),
                size: f["size"].as_u64(),
                modified: f["modified"].as_str().map(str::to_string),
                ..Default::default()
            }).collect())
        }
        _ => Ok(vec![]),
    }
}

/// Empreinte du certificat présenté par la sonde (premier contact, à faire confirmer).
pub async fn fetch_fingerprint(url: &str) -> Result<String, SourceError> {
    let parsed = url::Url::parse(url).map_err(|_| SourceError::invalid("URL invalide."))?;
    if parsed.scheme() != "https" { return Err(SourceError::invalid("URL https:// attendue.")); }
    let host = parsed.host_str().ok_or_else(|| SourceError::invalid("URL invalide."))?.to_string();
    let port = parsed.port().unwrap_or(443);
    let stream = tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| SourceError::new(ErrorCode::Timeout, format!("{url} : délai dépassé")))?
        .map_err(|e| SourceError::new(ErrorCode::Network, format!("{url} : {e}")))?;
    let config = crate::net::rustls_config(&crate::config::Tls { insecure: true, ..Default::default() })?;
    let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| SourceError::invalid("Hôte invalide."))?;
    let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|e| SourceError::new(ErrorCode::Tls, format!("{url} : {e}")))?;
    let cert = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).ok_or_else(|| SourceError::new(ErrorCode::Tls, "Aucun certificat présenté."))?;
    Ok(super::config::fingerprint(cert.as_ref()))
}

/// Décompression gzip au fil de l'eau.
struct Gunzip(Option<flate2::write::GzDecoder<Vec<u8>>>);

impl Gunzip {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        match &mut self.0 {
            None => Ok(chunk.to_vec()),
            Some(d) => {
                d.write_all(chunk)?;
                d.flush()?;
                Ok(std::mem::take(d.get_mut()))
            }
        }
    }
}

enum End {
    /// Fin normale (historique terminé, ou sonde qui ferme le flux avec un message).
    Done,
    /// Coupure : on reprend au curseur.
    Lost(String),
    Fatal(SourceError),
}

struct Run {
    connection: SondeConnection,
    token: String,
    source: String,
    file: Option<String>,
    options: StreamOptions,
    emit: Emit,
    timing: Timing,
    cancel: CancellationToken,
    cursor: Option<String>,
    live: bool,
}

/// Flux d'une source de sonde : `path` = [source] ou [source motif, fichier].
pub fn spawn(connection: SondeConnection, token: String, path: &[String], options: StreamOptions, emit: Emit, timing: Timing, cancel: CancellationToken) -> Result<(), SourceError> {
    let (source, file) = match path {
        [s] => (s.clone(), None),
        [s, f] => (s.clone(), Some(f.clone())),
        _ => return Err(SourceError::invalid("Chemin de sonde : [source] ou [source, fichier].")),
    };
    let run = Run { connection, token, source, file, options, emit, timing, cancel, cursor: None, live: false };
    tokio::spawn(run.main());
    Ok(())
}

impl Run {
    fn status(&self, state: StreamState, message: Option<String>) { (self.emit)(StreamEvent::status(state, message)); }

    async fn main(mut self) {
        (self.emit)(StreamEvent::status(StreamState::Connecting, None));
        let mut attempt = 0;
        loop {
            let cancel = self.cancel.clone();
            let end = tokio::select! {
                _ = cancel.cancelled() => return,
                end = self.connect() => end,
            };
            match end {
                End::Done => return,
                End::Fatal(e) => {
                    (self.emit)(StreamEvent::Status { state: StreamState::Error, message: Some(e.message.clone()), error: Some(e), replacements: None });
                    return;
                }
                End::Lost(why) => {
                    if !self.options.follow && self.cursor.is_none() {
                        let e = SourceError::new(ErrorCode::Network, why);
                        (self.emit)(StreamEvent::Status { state: StreamState::Error, message: Some(e.message.clone()), error: Some(e), replacements: None });
                        return;
                    }
                    self.status(StreamState::Reconnecting, Some(format!("Sonde injoignable ({why}) : reconnexion…")));
                    self.live = false;
                    let delay = self.timing.retry_delay(attempt);
                    attempt += 1;
                    tokio::select! {
                        _ = self.cancel.cancelled() => return,
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
            }
            if self.live { attempt = 0; }
        }
    }

    fn url_query(&self) -> Vec<(&'static str, String)> {
        let mut q = vec![];
        if let Some(f) = &self.file { q.push(("fichier", f.clone())); }
        if self.options.follow { q.push(("suivi", "1".into())); }
        match &self.cursor {
            Some(c) => q.push(("reprise", c.clone())),
            None => {
                if let Some(n) = self.options.tail_lines { q.push(("lignes", n.to_string())); }
                if let Some(s) = self.options.since_seconds { q.push(("depuis", s.to_string())); }
            }
        }
        q
    }

    async fn connect(&mut self) -> End {
        let id = percent_encoding::utf8_percent_encode(&self.source, percent_encoding::NON_ALPHANUMERIC).to_string();
        let request = match request(&self.connection, &self.token, &format!("/v1/sources/{id}/flux")) {
            Ok(r) => r.query(&self.url_query()).header("accept-encoding", "gzip"),
            Err(e) => return End::Fatal(e),
        };
        let response = match tokio::time::timeout(Duration::from_secs(20), request.send()).await {
            Err(_) => return End::Lost("délai dépassé".into()),
            Ok(Err(e)) => {
                let e = crate::net::from_reqwest(&e, &self.connection.url);
                return if e.code == ErrorCode::Tls { End::Fatal(e) } else { End::Lost(e.message) };
            }
            Ok(Ok(r)) => r,
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let e = refused(response).await;
            return if status >= 500 && status != 503 { End::Lost(e.message) } else { End::Fatal(e) };
        }
        let gzip = response.headers().get("content-encoding").is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"gzip"));
        let mut gunzip = Gunzip(gzip.then(|| flate2::write::GzDecoder::new(Vec::new())));
        let mut splitter = LineSplitter::default();
        let mut body = response.bytes_stream();
        loop {
            let chunk = match tokio::time::timeout(DEAD_AFTER.min(self.timing.idle_timeout), body.next()).await {
                Err(_) => return End::Lost("aucune nouvelle de la sonde".into()),
                Ok(None) => return if self.options.follow && self.live { End::Lost("connexion fermée".into()) } else { End::Done },
                Ok(Some(Err(e))) => return End::Lost(crate::net::from_reqwest(&e, &self.connection.url).message),
                Ok(Some(Ok(c))) => c,
            };
            let bytes = match gunzip.push(&chunk) {
                Ok(b) => b,
                Err(e) => return End::Lost(format!("flux illisible ({e})")),
            };
            for line in splitter.push(&bytes) {
                if line.trim().is_empty() { continue; }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(end) = self.event(v) { return end; }
            }
        }
    }

    /// Un événement de la sonde ; `Some` termine la connexion.
    fn event(&mut self, mut v: Value) -> Option<End> {
        match v["type"].as_str() {
            Some("ping") => None,
            Some("lines") => {
                if let Some(c) = v["curseur"].as_str() { self.cursor = Some(c.to_string()); }
                if let Some(map) = v.as_object_mut() { map.remove("curseur"); }
                if let Ok(e) = serde_json::from_value::<StreamEvent>(v) { (self.emit)(e); }
                None
            }
            Some("notice") => {
                if let Ok(e) = serde_json::from_value::<StreamEvent>(v) { (self.emit)(e); }
                None
            }
            Some("status") => {
                let state = v["state"].as_str().unwrap_or_default();
                match state {
                    // Reconnexion : la sonde repasse par « connecting », sans intérêt ici.
                    "connecting" => None,
                    "live" => {
                        self.live = true;
                        self.status(StreamState::Live, None);
                        None
                    }
                    "reconnecting" => {
                        self.status(StreamState::Reconnecting, v["message"].as_str().map(str::to_string));
                        None
                    }
                    "error" => {
                        let message = v["message"].as_str().unwrap_or("Erreur de la sonde").to_string();
                        let code = v["error"]["code"].as_str().and_then(|c| serde_json::from_value(Value::String(c.into())).ok()).unwrap_or(ErrorCode::Invalid);
                        Some(End::Fatal(SourceError::new(code, message)))
                    }
                    _ => {
                        self.status(StreamState::Ended, v["message"].as_str().map(str::to_string));
                        Some(End::Done)
                    }
                }
            }
            _ => None,
        }
    }
}
