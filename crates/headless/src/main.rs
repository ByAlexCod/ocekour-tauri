//! Ocekour sans interface.
//!
//!   ocekour-headless --mcp                         serveur MCP (stdio) pour un agent
//!   ocekour-headless --mcp-http [--mcp-port=N]     serveur MCP HTTP (127.0.0.1, jeton)
//!   ocekour-headless --notify                      notifications seules
//!   ocekour-headless cle init|generer|lire …       clés d'activation (administration)
//!   ocekour-headless fake-rancher [--port N]       faux Rancher de démonstration
//!
//! Option commune : --data-dir=<dossier> (sinon OCEKOUR_DATA_DIR, sinon %APPDATA%\Ocekour).

mod cle;

use ocekour_core::app::{App, AppOptions};
use ocekour_core::headless::{data_dir, parse_mode, run};
use std::sync::Arc;

const USAGE: &str = "Usage : ocekour-headless --mcp | --mcp-http [--mcp-port=N] | --notify | cle init|generer|lire | fake-rancher [--port N]
Options : --data-dir=<dossier>, --version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime tokio");
    let code = runtime.block_on(async move {
        match args.first().map(String::as_str) {
            Some("--version") | Some("-V") => {
                println!("ocekour-headless {}", env!("CARGO_PKG_VERSION"));
                0
            }
            Some("cle") => cle::main(&args[1..]),
            Some("fake-rancher") => fake_rancher(&args[1..]).await,
            _ => {
                let Some(mode) = parse_mode(&args) else {
                    eprintln!("{USAGE}");
                    return if args.iter().any(|a| a == "--help" || a == "-h") { 0 } else { 2 };
                };
                let dir = data_dir(&args);
                // En mode stdio, rien d'autre que le protocole sur la sortie standard.
                ocekour_core::logging::init(&dir, mode != ocekour_core::headless::Mode::Stdio);
                let app = match App::new(AppOptions::new(&dir, env!("CARGO_PKG_VERSION"), Arc::new(ocekour_core::ui::NoUi))) {
                    Ok(app) => app,
                    Err(e) => {
                        eprintln!("Démarrage impossible : {e}");
                        return 1;
                    }
                };
                run(app, mode).await
            }
        }
    });
    std::process::exit(code);
}

/// Faux Rancher (pods, logs en continu, redémarrages) pour une démonstration sans cluster.
async fn fake_rancher(args: &[String]) -> i32 {
    let port = args.windows(2).find(|w| w[0] == "--port").and_then(|w| w[1].parse().ok()).unwrap_or(0);
    let server = ocekour_core::testing::fake_rancher::start(port).await;
    println!("Faux Rancher : {}", server.url);
    println!("Jeton : {}", ocekour_core::testing::fake_rancher::FAKE_TOKEN);
    println!("Certificat (cochez « Ignorer la vérification SSL », ou enregistrez ce PEM) :\n{}", server.ca_pem);
    let _ = tokio::signal::ctrl_c().await;
    0
}
