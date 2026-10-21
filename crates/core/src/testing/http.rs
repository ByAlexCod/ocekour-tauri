//! Petit serveur HTTP(S) de test : un gestionnaire reçoit la requête et rend une réponse
//! (JSON, texte, ou flux de morceaux).

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Requête reçue.
#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    /// Chemin décodé.
    pub path: String,
    /// Chemin brut (encodage conservé, ex. `%2F`).
    pub raw_path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn json(&self) -> Value { serde_json::from_slice(&self.body).unwrap_or(Value::Null) }
    pub fn header(&self, name: &str) -> Option<&str> { self.headers.get(&name.to_lowercase()).map(String::as_str) }
    pub fn q(&self, name: &str) -> Option<&str> { self.query.get(name).map(String::as_str) }
}

/// Réponse : morceaux envoyés au fil de l'eau ; une erreur coupe la connexion.
pub type Chunks = mpsc::UnboundedReceiver<Result<Bytes, std::io::Error>>;

pub enum Resp {
    Json(u16, Value),
    Text(u16, String),
    Bytes(u16, Vec<u8>, Vec<(String, String)>),
    Stream(u16, String, Chunks),
    Empty(u16, Vec<(String, String)>),
}

pub type Handler = Arc<dyn Fn(Req) -> Pin<Box<dyn Future<Output = Resp> + Send>> + Send + Sync>;

pub fn handler<F, Fut>(f: F) -> Handler
where
    F: Fn(Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Resp> + Send + 'static,
{
    Arc::new(move |r| Box::pin(f(r)))
}

pub struct TestServer {
    pub url: String,
    pub port: u16,
    shutdown: tokio_util::sync::CancellationToken,
}

impl Drop for TestServer {
    fn drop(&mut self) { self.shutdown.cancel(); }
}

/// Certificat auto-signé (`localhost`, `127.0.0.1`) : PEM du certificat et de la clé.
pub fn self_signed(name: &str) -> (String, String, Vec<u8>) {
    let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
    params.distinguished_name.push(rcgen::DnType::CommonName, name);
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName("localhost".try_into().unwrap()),
        rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap()),
    ];
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    (cert.pem(), key.serialize_pem(), cert.der().to_vec())
}

fn respond(resp: Resp) -> hyper::Response<BoxBody<Bytes, std::io::Error>> {
    let full = |b: Vec<u8>| Full::new(Bytes::from(b)).map_err(|e: Infallible| match e {}).boxed();
    let mut builder = hyper::Response::builder();
    let body = match resp {
        Resp::Json(status, v) => {
            builder = builder.status(status).header("content-type", "application/json");
            full(v.to_string().into_bytes())
        }
        Resp::Text(status, t) => {
            builder = builder.status(status).header("content-type", "text/plain; charset=utf-8");
            full(t.into_bytes())
        }
        Resp::Bytes(status, b, headers) => {
            builder = builder.status(status);
            for (k, v) in headers { builder = builder.header(k, v); }
            full(b)
        }
        Resp::Empty(status, headers) => {
            builder = builder.status(status);
            for (k, v) in headers { builder = builder.header(k, v); }
            full(vec![])
        }
        Resp::Stream(status, content_type, rx) => {
            builder = builder.status(status).header("content-type", content_type);
            let stream = tokio_stream_from(rx).map(|r| r.map(Frame::data));
            BodyExt::boxed(StreamBody::new(stream))
        }
    };
    builder.body(body).unwrap()
}

fn tokio_stream_from(mut rx: Chunks) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

/// Démarre le serveur sur 127.0.0.1 (port libre si 0), en HTTPS si `tls` = (cert, clé) PEM.
pub async fn start(port: u16, tls: Option<(String, String)>, handler: Handler) -> TestServer {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = tls.map(|(cert, key)| {
        let certs = rustls_pemfile::certs(&mut cert.as_bytes()).collect::<Result<Vec<_>, _>>().unwrap();
        let key = rustls_pemfile::private_key(&mut key.as_bytes()).unwrap().unwrap();
        let config = rustls::ServerConfig::builder_with_provider(crate::net::provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        tokio_rustls::TlsAcceptor::from(Arc::new(config))
    });
    let scheme = if acceptor.is_some() { "https" } else { "http" };
    let shutdown = tokio_util::sync::CancellationToken::new();
    let stop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = stop.cancelled() => break,
                a = listener.accept() => a,
            };
            let Ok((stream, _)) = accepted else { continue };
            let handler = handler.clone();
            let acceptor = acceptor.clone();
            let stop = stop.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let handler = handler.clone();
                    async move {
                        let (parts, body) = req.into_parts();
                        let body = body.collect().await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
                        let raw_path = parts.uri.path().to_string();
                        let query = parts
                            .uri
                            .query()
                            .map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect())
                            .unwrap_or_default();
                        let headers = parts
                            .headers
                            .iter()
                            .map(|(k, v)| (k.as_str().to_lowercase(), v.to_str().unwrap_or_default().to_string()))
                            .collect();
                        let path = percent_encoding::percent_decode_str(&raw_path).decode_utf8_lossy().to_string();
                        let r = Req { method: parts.method.to_string(), path, raw_path, query, headers, body };
                        Ok::<_, Infallible>(respond(handler(r).await))
                    }
                });
                match acceptor {
                    Some(a) => {
                        if let Ok(tls) = a.accept(stream).await {
                            let conn = http1::Builder::new().serve_connection(TokioIo::new(tls), service);
                            tokio::select! { _ = conn => {}, _ = stop.cancelled() => {} }
                        }
                    }
                    None => {
                        let conn = http1::Builder::new().serve_connection(TokioIo::new(stream), service);
                        tokio::select! { _ = conn => {}, _ = stop.cancelled() => {} }
                    }
                }
            });
        }
    });
    TestServer { url: format!("{scheme}://127.0.0.1:{port}"), port, shutdown }
}

/// Runtime partagé pour les serveurs de test qui doivent survivre à un test (fixtures).
pub fn shared_runtime() -> &'static tokio::runtime::Runtime {
    static RT: once_cell::sync::Lazy<tokio::runtime::Runtime> = once_cell::sync::Lazy::new(|| {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
    });
    &RT
}

/// Comme `start`, sur le runtime partagé.
pub async fn start_shared(port: u16, tls: Option<(String, String)>, handler: Handler) -> TestServer {
    shared_runtime().spawn(start(port, tls, handler)).await.unwrap()
}
