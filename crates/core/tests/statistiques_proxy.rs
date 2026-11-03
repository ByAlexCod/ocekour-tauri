//! Statistiques (Matomo) sur un réseau qui impose le proxy du système : l'envoi direct échoue,
//! le même envoi par le proxy passe ; sinon la cause est gardée pour l'écran de blocage.
//! Test à part (son propre processus) : il fixe les variables de proxy de l'environnement.

use ocekour_core::analytics::{Analytics, AnalyticsEvent, Options, Target};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Proxy HTTP de test : garde l'en-tête de chaque requête et répond `status` (204 par défaut).
async fn proxy(seen: Arc<Mutex<Vec<String>>>, status: Arc<AtomicU16>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { break };
            let (seen, status) = (seen.clone(), status.clone());
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                // En-tête puis corps (content-length).
                loop {
                    let Ok(n) = stream.read(&mut buf).await else { return };
                    if n == 0 { return; }
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    let Some(end) = text.find("\r\n\r\n") else { continue };
                    let length = text[..end].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if raw.len() >= end + 4 + length {
                        seen.lock().push(text[..end].to_string());
                        break;
                    }
                }
                let code = status.load(Ordering::SeqCst);
                let reason = if code == 204 { "No Content" } else { "Forbidden" };
                let _ = stream.write_all(format!("HTTP/1.1 {code} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await;
            });
        }
    });
    port
}

fn analytics(dir: &std::path::Path) -> Arc<Analytics> {
    Analytics::new(Options {
        // Nom réservé (RFC 2606) : jamais résolu en direct, seul le proxy « le connaît ».
        target: Box::new(|| Some(Target { url: "http://matomo.ocekour.invalid/".into(), site_id: 3, insecure: false, dim_version: None, dim_key: None, key_id: None })),
        visitor_file: dir.join("visiteur"),
        version: "2.2.0".into(),
        user_agent: "Mozilla/5.0 Ocekour/2.2.0".into(),
        language: "fr-FR".into(),
        resolution: None,
        flush: Duration::from_secs(30),
        heartbeat: Duration::from_secs(300),
        on_delivery: None,
    })
}

#[tokio::test]
async fn envoi_par_le_proxy_du_systeme() {
    let seen = Arc::new(Mutex::new(vec![]));
    let status = Arc::new(AtomicU16::new(204));
    let port = proxy(seen.clone(), status.clone()).await;
    // SAFETY : avant tout client HTTP de ce processus ; un seul test dans ce binaire.
    unsafe {
        for var in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] { std::env::set_var(var, format!("http://127.0.0.1:{port}")); }
        for var in ["NO_PROXY", "no_proxy"] { std::env::remove_var(var); }
    }
    let dir = tempfile::tempdir().unwrap();
    let stats = analytics(dir.path());

    // Direct impossible (DNS), par le proxy : envoyé.
    stats.event(&AnalyticsEvent::new("Application", "Démarrage", None));
    assert!(stats.check().await, "envoi par le proxy attendu : {:?}", stats.last_error());
    assert_eq!(stats.last_error(), None);
    let request = seen.lock().last().cloned().unwrap();
    assert!(request.starts_with("POST http://matomo.ocekour.invalid/matomo.php "), "{request}");
    assert!(request.to_ascii_lowercase().contains("user-agent: mozilla/5.0 ocekour/2.2.0"), "{request}");

    // Le proxy refuse : échec, avec la cause des deux voies pour l'écran de blocage.
    status.store(403, Ordering::SeqCst);
    assert!(!stats.check().await);
    let error = stats.last_error().expect("cause gardée");
    assert!(error.contains("HTTP 403"), "{error}");
    assert!(error.contains("matomo.ocekour.invalid"), "la voie directe est aussi expliquée : {error}");

    // Le proxy accepte de nouveau : la file gardée part, l'erreur est effacée.
    status.store(204, Ordering::SeqCst);
    let before = seen.lock().len();
    assert!(stats.check().await);
    assert_eq!(stats.last_error(), None);
    assert_eq!(seen.lock().len(), before + 1, "la voie qui a marché (proxy) est essayée en premier");
}
