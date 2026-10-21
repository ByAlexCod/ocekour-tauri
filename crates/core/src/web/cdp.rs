//! Client Chrome DevTools Protocol minimal (WebSocket, sessions « à plat »).

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;
pub type EventHandler = Arc<dyn Fn(&str, &Value, Option<&str>) + Send + Sync>;

pub struct Cdp {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: AtomicU64,
    closed: CancellationToken,
}

impl Cdp {
    /// Connexion au WebSocket du navigateur ; `on_event(méthode, params, session)`.
    pub async fn connect(ws_url: &str, on_event: EventHandler) -> anyhow::Result<Arc<Cdp>> {
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(None).max_frame_size(None);
        let (ws, _) = tokio_tungstenite::connect_async_with_config(ws_url, Some(config), false).await?;
        let (mut sink, mut stream) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let pending: Pending = Arc::default();
        let closed = CancellationToken::new();
        {
            let closed = closed.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = closed.cancelled() => break,
                        m = rx.recv() => match m {
                            Some(text) => if sink.send(Message::Text(text.into())).await.is_err() { break },
                            None => break,
                        },
                    }
                }
                let _ = sink.close().await;
            });
        }
        {
            let (pending, closed) = (pending.clone(), closed.clone());
            tokio::spawn(async move {
                while let Some(Ok(message)) = stream.next().await {
                    let Message::Text(text) = message else { continue };
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    if let Some(id) = v["id"].as_u64() {
                        if let Some(reply) = pending.lock().remove(&id) {
                            let result = match v.get("error") {
                                Some(e) => Err(e["message"].as_str().unwrap_or("erreur CDP").to_string()),
                                None => Ok(v["result"].clone()),
                            };
                            let _ = reply.send(result);
                        }
                    } else if let Some(method) = v["method"].as_str() {
                        on_event(method, &v["params"], v["sessionId"].as_str());
                    }
                }
                closed.cancel();
                for (_, reply) in pending.lock().drain() { let _ = reply.send(Err("Navigateur fermé.".into())); }
            });
        }
        Ok(Arc::new(Cdp { tx, pending, next: AtomicU64::new(1), closed }))
    }

    pub fn is_closed(&self) -> bool { self.closed.is_cancelled() }

    /// Appel d'une méthode (dans une session de page si `session`), borné par `timeout`.
    pub async fn call(&self, method: &str, params: Value, session: Option<&str>, timeout: Duration) -> anyhow::Result<Value> {
        if self.is_closed() { anyhow::bail!("Navigateur fermé."); }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session { message["sessionId"] = json!(s); }
        let (reply, wait) = oneshot::channel();
        self.pending.lock().insert(id, reply);
        if self.tx.send(message.to_string()).is_err() {
            self.pending.lock().remove(&id);
            anyhow::bail!("Navigateur fermé.");
        }
        match tokio::time::timeout(timeout, wait).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => anyhow::bail!("{method} : {e}"),
            Ok(Err(_)) => anyhow::bail!("Navigateur fermé."),
            Err(_) => {
                self.pending.lock().remove(&id);
                anyhow::bail!("{method} : délai de {} ms dépassé", timeout.as_millis())
            }
        }
    }

    pub fn close(&self) { self.closed.cancel(); }
}
