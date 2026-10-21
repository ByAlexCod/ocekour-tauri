//! Serveur MCP HTTP (Streamable HTTP, sans session) sur 127.0.0.1 uniquement, protégé par
//! un jeton (`Authorization: Bearer …`) et contre le DNS rebinding (hôte vérifié).

use super::{Session, rpc_error};
use crate::app::App;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_MCP_PORT: u16 = 47110;

/// Corps d'une requête au plus (10 Mo).
const MAX_BODY: usize = 10 * 1024 * 1024;

pub type TokenFn = Arc<dyn Fn() -> String + Send + Sync>;

struct Running {
    port: u16,
    stop: CancellationToken,
}

pub struct McpHttpServer {
    token: TokenFn,
    running: Mutex<Option<Running>>,
}

fn same_token(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn reply(status: StatusCode, body: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(&body).unwrap_or_default())))
        .unwrap()
}

fn error(status: StatusCode, message: &str) -> Response<Full<Bytes>> {
    reply(status, json!({ "jsonrpc": "2.0", "error": { "code": -32000, "message": message }, "id": null }))
}

impl McpHttpServer {
    pub fn new(token: TokenFn) -> Self { McpHttpServer { token, running: Mutex::new(None) } }

    pub fn running(&self) -> bool { self.running.lock().is_some() }

    pub fn port(&self) -> Option<u16> { self.running.lock().as_ref().map(|r| r.port) }

    pub fn url(&self) -> String { format!("http://127.0.0.1:{}/mcp", self.port().unwrap_or(0)) }

    /// Démarre (ou redémarre sur un autre port). Port 0 : port libre choisi par le système.
    pub async fn start(&self, app: Arc<App>, port: u16) -> anyhow::Result<()> {
        if self.port().is_some_and(|p| p == port && port != 0) { return Ok(()); }
        self.stop();
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
        let port = listener.local_addr()?.port();
        let stop = CancellationToken::new();
        *self.running.lock() = Some(Running { port, stop: stop.clone() });
        log::info!("Serveur MCP (HTTP) : http://127.0.0.1:{port}/mcp");
        let token = self.token.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = tokio::select! {
                    _ = stop.cancelled() => break,
                    accepted = listener.accept() => match accepted { Ok(s) => s, Err(_) => continue },
                };
                let (app, token, stop) = (app.clone(), token.clone(), stop.clone());
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        let (app, token) = (app.clone(), token.clone());
                        async move { Ok::<_, std::convert::Infallible>(handle(app, token, port, req).await) }
                    });
                    let conn = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), service);
                    tokio::pin!(conn);
                    tokio::select! {
                        _ = conn.as_mut() => {}
                        _ = stop.cancelled() => conn.as_mut().graceful_shutdown(),
                    }
                });
            }
        });
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            r.stop.cancel();
            log::info!("Serveur MCP (HTTP) arrêté");
        }
    }
}

async fn handle(app: Arc<App>, token: TokenFn, port: u16, req: Request<Incoming>) -> Response<Full<Bytes>> {
    if req.uri().path() != "/mcp" { return error(StatusCode::NOT_FOUND, "Chemin : /mcp"); }
    let host = req.headers().get("host").and_then(|h| h.to_str().ok()).unwrap_or_default();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return error(StatusCode::FORBIDDEN, "Hôte refusé");
    }
    let auth = req.headers().get("authorization").and_then(|h| h.to_str().ok()).unwrap_or_default();
    match auth.strip_prefix("Bearer ") {
        Some(given) if same_token(given, &token()) => {}
        _ => return error(StatusCode::UNAUTHORIZED, "Jeton MCP absent ou invalide (Paramètres → Serveur MCP)"),
    }
    if req.method() != hyper::Method::POST { return error(StatusCode::METHOD_NOT_ALLOWED, "Méthode non prise en charge (POST)"); }
    let user_agent = req.headers().get("user-agent").and_then(|h| h.to_str().ok()).map(str::to_string);
    let body = match http_body_util::Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Requête trop volumineuse"),
    };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(m) => m,
        Err(_) => return reply(StatusCode::BAD_REQUEST, rpc_error(Value::Null, -32700, "Parse error")),
    };
    // Sans session : une session par requête.
    let mut session = Session::new(app);
    session.user_agent = user_agent;
    match session.handle_any(message).await {
        Some(r) => reply(StatusCode::OK, r),
        None => Response::builder().status(StatusCode::ACCEPTED).body(Full::new(Bytes::new())).unwrap(),
    }
}
