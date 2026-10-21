//! Mode sans interface `--mcp` : protocole JSON-RPC sur l'entrée / la sortie standard, un
//! message par ligne. Rien d'autre n'est écrit sur la sortie standard.

use super::{Session, rpc_error};
use crate::app::App;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

/// Sert le protocole jusqu'à la fermeture de l'entrée (client parti).
pub async fn serve<R, W>(app: Arc<App>, input: R, output: W) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let output = Arc::new(Mutex::new(output));
    let write = {
        let output = output.clone();
        move |v: Value| {
            let output = output.clone();
            async move {
                let mut line = serde_json::to_vec(&v).unwrap_or_default();
                line.push(b'\n');
                let mut o = output.lock().await;
                let _ = o.write_all(&line).await;
                let _ = o.flush().await;
            }
        }
    };

    // Service activé, jeton ajouté, option web (dé)cochée… : la liste des outils change.
    let (tx, mut changes) = mpsc::unbounded_channel::<()>();
    {
        let tx2 = tx.clone();
        app.config.on_changed(move |_| { let _ = tx2.send(()); });
        app.accounts.on_changed(move || { let _ = tx.send(()); });
    }
    let session = Arc::new(Mutex::new(Session::new(app.clone())));
    let watcher = {
        let (session, write) = (session.clone(), write.clone());
        tokio::spawn(async move {
            let mut visible = session.lock().await.visible_names();
            while changes.recv().await.is_some() {
                let now = session.lock().await.visible_names();
                if now != visible {
                    visible = now;
                    write(json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })).await;
                }
            }
        })
    };

    log::info!("Serveur MCP (stdio) prêt");
    let mut tasks = tokio::task::JoinSet::new();
    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() { continue; }
        let message: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => {
                write(rpc_error(Value::Null, -32700, "Parse error")).await;
                continue;
            }
        };
        if message["method"] == "initialize" {
            // Initialisation : traitée tout de suite (nom du client pour le journal).
            let reply = session.lock().await.handle_any(message).await;
            if let Some(r) = reply { write(r).await; }
            continue;
        }
        // Les autres requêtes en parallèle (un outil long n'en bloque pas d'autres).
        let mut s = session.lock().await.clone();
        let write = write.clone();
        while tasks.try_join_next().is_some() {}
        tasks.spawn(async move {
            if let Some(r) = s.handle_any(message).await { write(r).await; }
        });
    }
    // Réponses en cours envoyées avant de rendre la main.
    while tasks.join_next().await.is_some() {}
    watcher.abort();
    log::info!("Serveur MCP (stdio) : entrée fermée");
    Ok(())
}
