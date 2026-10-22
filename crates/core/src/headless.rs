//! Modes sans interface, communs à `ocekour-headless` et à l'exe Tauri :
//! `--mcp` (stdio), `--mcp-http [--mcp-port=N]`, `--notify` (démon de notifications).

use crate::app::App;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// Protocole MCP sur l'entrée / la sortie standard.
    Stdio,
    /// Serveur MCP HTTP (port de config.yaml, sinon celui donné).
    Http(Option<u16>),
    /// Notifications seules : ni fenêtre ni serveur MCP.
    Notify,
}

/// Mode demandé par la ligne de commande, s'il y en a un.
pub fn parse_mode(args: &[String]) -> Option<Mode> {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--mcp") { return Some(Mode::Stdio); }
    if has("--mcp-http") {
        let port = args.iter().find_map(|a| a.strip_prefix("--mcp-port=")).and_then(|p| p.parse::<u16>().ok()).filter(|p| *p > 0);
        return Some(Mode::Http(port));
    }
    if has("--notify") { return Some(Mode::Notify); }
    None
}

/// Dossier de données : `--data-dir=…`, sinon `OCEKOUR_DATA_DIR`, sinon %APPDATA%\Ocekour.
pub fn data_dir(args: &[String]) -> std::path::PathBuf {
    args.iter().find_map(|a| a.strip_prefix("--data-dir=")).map(Into::into).unwrap_or_else(crate::app::default_data_dir)
}

async fn until_stopped() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

/// Exécute un mode jusqu'à son terme ; renvoie le code de sortie.
pub async fn run(app: Arc<App>, mode: Mode) -> i32 {
    // config.yaml modifié à la main : pris en compte sans redémarrer (outils MCP, notifications).
    let _watcher = app.config.watch().map_err(|e| log::warn!("Surveillance de config.yaml impossible : {e}")).ok();
    let code = match mode {
        Mode::Stdio => {
            // Client parti (entrée fermée) : Ocekour s'arrête aussi, sans rester orphelin.
            let loading = { let app = app.clone(); tokio::spawn(async move { app.licence_loaded().await }) };
            let result = crate::mcp::stdio::serve(app.clone(), tokio::io::stdin(), tokio::io::stdout()).await;
            loading.abort();
            if let Err(e) = result { log::error!("Serveur MCP (stdio) : {e}"); 1 } else { 0 }
        }
        Mode::Http(port) => {
            app.licence_loaded().await;
            let port = port.unwrap_or(app.config.config().mcp_port);
            match app.mcp_http.start(app.clone(), port).await {
                Ok(()) => {
                    eprintln!("Serveur MCP : {}\nJeton : {}", app.mcp_http.url(), app.mcp_token(false));
                    until_stopped().await;
                    0
                }
                Err(e) => {
                    log::error!("Serveur MCP : port {port} indisponible : {e}");
                    eprintln!("Serveur MCP : port {port} indisponible : {e}");
                    1
                }
            }
        }
        Mode::Notify => {
            app.licence_loaded().await;
            if !app.config.config().notifications.enabled {
                log::warn!("--notify : les notifications sont désactivées (Paramètres → Notifications).");
            }
            app.start_background();
            until_stopped().await;
            0
        }
    };
    app.shutdown().await;
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn modes() {
        assert_eq!(parse_mode(&a(&["x", "--mcp"])), Some(Mode::Stdio));
        assert_eq!(parse_mode(&a(&["--mcp-http"])), Some(Mode::Http(None)));
        assert_eq!(parse_mode(&a(&["--mcp-http", "--mcp-port=4000"])), Some(Mode::Http(Some(4000))));
        assert_eq!(parse_mode(&a(&["--mcp-http", "--mcp-port=abc"])), Some(Mode::Http(None)));
        assert_eq!(parse_mode(&a(&["--notify"])), Some(Mode::Notify));
        assert_eq!(parse_mode(&a(&[])), None);
        assert_eq!(data_dir(&a(&["--data-dir=/tmp/x"])), std::path::PathBuf::from("/tmp/x"));
    }
}
