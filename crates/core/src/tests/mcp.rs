//! Serveur MCP : outils de l'interface (faux Rancher), licence, transports stdio et HTTP.

use super::support::{TestApp, assert_match, call, names, session, test_app};
use crate::testing::fake_rancher::{self, FAKE_TOKEN, FakeRancher, HEALTHY};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;

async fn setup(activate: bool) -> (FakeRancher, TestApp, String) {
    let rancher = fake_rancher::start(0).await;
    rancher.control.interval_ms.store(20, Ordering::SeqCst);
    let t = test_app("", activate).await;
    let id = t.app.connections.save(&json!({
        "type": "rancher", "name": "Rancher test", "url": rancher.url, "secret": FAKE_TOKEN,
        "tls": { "insecure": true }, "clusters": [{ "id": "c-prod1", "name": "prod" }],
    })).unwrap();
    (rancher, t, id)
}

fn container() -> Value { json!(["c-prod1", "app-commandes", HEALTHY, "app"]) }

#[tokio::test(flavor = "multi_thread")]
async fn outils_de_l_interface() {
    let (_rancher, t, id) = setup(true).await;
    let app = &t.app;

    // Les actions de l'interface, avec annotations de lecture / destruction.
    let list = session(app).await.handle(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await.unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    let all: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["licence_etat", "licence_activer", "connexions_lister", "connexion_enregistrer", "sources_parcourir", "logs_lire", "logs_exporter", "profil_modifier", "s3_lister", "s3_supprimer", "lien_ouvrir", "lien_creer", "attendre"] {
        assert!(all.contains(&n), "{n}");
    }
    let by = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap().clone();
    assert_eq!(by("logs_lire")["annotations"]["readOnlyHint"], true);
    assert_eq!(by("s3_supprimer")["annotations"]["destructiveHint"], true);
    assert_eq!(by("logs_lire")["inputSchema"]["type"], "object");

    // Connexions sans secret.
    let c = call(app, "connexions_lister", json!({})).await;
    assert_match(&c.value, &json!([{ "id": id, "type": "rancher", "secretEnregistre": true }]));
    assert!(!c.text.contains(FAKE_TOKEN));

    // Parcours Rancher et logs analysés.
    let pods = call(app, "sources_parcourir", json!({ "connexion": id, "chemin": ["c-prod1", "app-commandes"] })).await;
    assert!(pods.value.as_array().unwrap().iter().any(|p| p["id"] == HEALTHY), "{}", pods.text);
    let logs = call(app, "logs_lire", json!({ "connexion": id, "chemin": container(), "lignes": 20, "max": 5 })).await;
    assert_match(&logs.value, &json!({ "etat": "ended", "renvoyees": 5, "profil": "prod/app-commandes/api-commandes" }));
    let first = &logs.value["lignes"][0];
    assert!(first["heure"].is_string() && first["niveau"].is_string(), "{first}");

    // Recherche.
    let found = call(app, "logs_lire", json!({ "connexion": id, "chemin": container(), "lignes": 50, "recherche": "zzz-introuvable" })).await;
    assert_eq!(found.value["renvoyees"], 0);

    // Export dans un fichier (refuse d'écraser sans accord).
    let file = t.path("export.ndjson");
    let args = json!({ "connexion": id, "chemin": container(), "lignes": 10, "fichier": file });
    let e = call(app, "logs_exporter", args.clone()).await;
    assert_match(&e.value, &json!({ "format": "ndjson", "lignes": 10 }));
    assert_eq!(std::fs::read_to_string(&file).unwrap().trim().lines().count(), 10);
    assert!(call(app, "logs_exporter", args).await.error);
    assert!(call(app, "logs_exporter", json!({ "connexion": id, "chemin": container(), "fichier": "relatif.log" })).await.error);

    // Lien de partage créé puis relu.
    let link = call(app, "lien_creer", json!({ "connexion": id, "chemin": container(), "recherche": "paiement" })).await;
    let link = link.value.as_str().unwrap().to_string();
    assert!(link.starts_with("ocekour://open?d="), "{link}");
    let opened = call(app, "lien_ouvrir", json!({ "lien": link })).await;
    assert_match(&opened.value, &json!({ "connexion": id, "chemin": container(), "recherche": { "text": "paiement" } }));

    // Profils : modification en un clic.
    let p = call(app, "profil_modifier", json!({ "match": "prod/app-commandes/api-commandes", "operation": { "op": "toggleColumn", "path": "user.id" } })).await;
    assert_match(&p.value, &json!({ "match": "prod/app-commandes/api-commandes", "columns": ["user.id"] }));

    // Suppression : confirmation obligatoire (validation des arguments).
    let refused = call(app, "connexion_supprimer", json!({ "connexion": id })).await;
    assert!(refused.error && refused.text.contains("Invalid arguments"), "{}", refused.text);
    assert_eq!(app.config.config().connections.len(), 1);

    // Attendre : respecte le délai et le borne.
    let start = std::time::Instant::now();
    let w = call(app, "attendre", json!({ "secondes": 1 })).await;
    assert!(!w.error && start.elapsed().as_millis() >= 950);
    assert_match(&w.value, &json!({ "attendu": 1 }));
    assert!(call(app, "attendre", json!({ "secondes": 0 })).await.error);
    assert!(call(app, "attendre", json!({ "minutes": 6 })).await.error);

    // Méthode inconnue, outil inconnu.
    let mut s = session(app).await;
    assert_eq!(s.handle(json!({ "jsonrpc": "2.0", "id": 9, "method": "resources/list" })).await.unwrap()["error"]["code"], -32601);
    assert_eq!(s.handle(json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": "rien" } })).await.unwrap()["error"]["code"], -32602);
    assert!(s.handle(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await.is_none());

    // Suppression confirmée.
    let done = call(app, "connexion_supprimer", json!({ "connexion": id, "confirmer": true })).await;
    assert!(!done.error, "{}", done.text);
    assert!(app.config.config().connections.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sans_activation_seuls_les_outils_de_licence() {
    let (_rancher, t, _) = setup(false).await;
    let app = &t.app;
    let c = call(app, "connexions_lister", json!({})).await;
    assert!(c.error && c.text.contains("licence_activer"), "{}", c.text);
    assert_match(&call(app, "licence_etat", json!({})).await.value, &json!({ "status": "missing" }));
    // Activation par l'outil, puis tout répond.
    let bad = call(app, "licence_activer", json!({ "cle": "OCEKOUR1-invalide" })).await;
    assert!(bad.error, "{}", bad.text);
    let ok = call(app, "licence_activer", json!({ "cle": t.key })).await;
    assert_match(&ok.value, &json!({ "status": "active", "development": true }));
    assert!(!call(app, "connexions_lister", json!({})).await.error);
}

#[tokio::test(flavor = "multi_thread")]
async fn serveur_http() {
    let t = test_app("", true).await;
    let server = crate::mcp::http::McpHttpServer::new(Arc::new(|| "jeton-secret".to_string()));
    server.start(t.app.clone(), 0).await.unwrap();
    let url = server.url();
    assert!(regex::Regex::new(r"^http://127\.0\.0\.1:\d+/mcp$").unwrap().is_match(&url), "{url}");
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let no_token = client.post(&url).json(&body).send().await.unwrap();
    assert_eq!(no_token.status(), 401);
    let bad_token = client.post(&url).bearer_auth("autre").json(&body).send().await.unwrap();
    assert_eq!(bad_token.status(), 401);
    let good = client.post(&url).bearer_auth("jeton-secret").json(&body).send().await.unwrap();
    assert_eq!(good.status(), 200);
    assert!(good.text().await.unwrap().contains("logs_lire"));
    let get = client.get(&url).bearer_auth("jeton-secret").send().await.unwrap();
    assert_eq!(get.status(), 405);
    let other_path = client.post(url.replace("/mcp", "/autre")).bearer_auth("jeton-secret").json(&body).send().await.unwrap();
    assert_eq!(other_path.status(), 404);
    // Notification seule : 202 sans corps.
    let notif = client.post(&url).bearer_auth("jeton-secret").json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).send().await.unwrap();
    assert_eq!(notif.status(), 202);
    // Lot de requêtes.
    let batch = client.post(&url).bearer_auth("jeton-secret").json(&json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }, { "jsonrpc": "2.0", "id": 2, "method": "ping" }])).send().await.unwrap();
    assert_eq!(batch.json::<Value>().await.unwrap().as_array().unwrap().len(), 2);
    // DNS rebinding : un autre nom d'hôte est refusé, même avec le jeton.
    let port = server.port().unwrap();
    let raw = format!(
        "POST /mcp HTTP/1.1\r\nhost: evil.example:80\r\nauthorization: Bearer jeton-secret\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.to_string().len(), body
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    tcp.write_all(raw.as_bytes()).await.unwrap();
    let mut response = String::new();
    tcp.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    server.stop();
    assert!(!server.running());
    assert!(client.post(&url).bearer_auth("jeton-secret").json(&body).send().await.is_err());
}

async fn send(w: &mut (impl tokio::io::AsyncWrite + Unpin), v: Value) {
    use tokio::io::AsyncWriteExt;
    w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn transport_stdio() {
    let t = test_app("", true).await;
    let (client_side, server_side) = tokio::io::duplex(1 << 16);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server = tokio::spawn(crate::mcp::stdio::serve(t.app.clone(), server_read, server_write));
    let (client_read, mut client_write) = tokio::io::split(client_side);
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut lines = BufReader::new(client_read).lines();
    send(&mut client_write, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05", "clientInfo": { "name": "cli", "version": "1" }, "capabilities": {} } })).await;
    let init: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init["result"]["serverInfo"]["name"], "ocekour");
    assert!(init["result"]["instructions"].as_str().unwrap().contains("connexions_lister"));
    send(&mut client_write, json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    send(&mut client_write, Value::String("pas un objet".into())).await;
    let err: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(err["error"]["code"], -32600);
    client_write.write_all(b"{ illisible\n").await.unwrap();
    let err: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(err["error"]["code"], -32700);
    send(&mut client_write, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "connexions_lister", "arguments": {} } })).await;
    let r: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(r["id"], 2);
    assert_eq!(r["result"]["content"][0]["text"], "[]");

    // Service activé : la liste des outils change (notification).
    t.app.secrets.set("service:jira", "jeton").unwrap();
    t.app.accounts.save("jira", "https://jira.intra", &crate::config::Tls::default()).unwrap();
    let n: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(n, json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }));
    assert!(names(&t.app).await.contains(&"jira_chercher".to_string()));
    // Entrée fermée : le serveur s'arrête.
    drop(client_write);
    drop(lines);
    tokio::time::timeout(std::time::Duration::from_secs(5), server).await.unwrap().unwrap().unwrap();
}
