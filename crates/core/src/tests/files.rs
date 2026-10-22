//! Fichiers du poste : canaux de l'interface (récents, favoris, encodage, flux) et outils MCP
//! limités aux fichiers ouverts aux agents.

use super::support::{TestApp, assert_match, call, names, test_app_with};
use crate::api::Caller;
use crate::ui::{OpenOptions, Ui};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
struct Window {
    events: Mutex<Vec<Value>>,
    picked: Mutex<Vec<PathBuf>>,
}

#[async_trait::async_trait]
impl Ui for Window {
    fn emit_to(&self, _: &str, event: &str, payload: Value) {
        if event == "streams:event" { self.events.lock().push(payload); }
    }
    async fn open_dialog(&self, _: OpenOptions) -> Vec<PathBuf> { self.picked.lock().clone() }
}

async fn c(t: &TestApp, channel: &str, args: Value) -> Result<Value, String> {
    crate::api::call(&t.app, &Caller { window: "main".into() }, channel, args.as_array().cloned().unwrap_or_default()).await
}

/// Lignes reçues par la fenêtre pour un flux.
fn lines_of(ui: &Window, id: &str) -> Vec<String> {
    ui.events.lock().iter().filter(|e| e["id"] == id && e["event"]["type"] == "lines")
        .flat_map(|e| e["event"]["lines"].as_array().cloned().unwrap_or_default())
        .map(|l| l["text"].as_str().unwrap_or_default().to_string()).collect()
}

async fn wait_for(mut ok: impl FnMut() -> bool) {
    for _ in 0..250 {
        if ok() { return; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("délai dépassé");
}

#[tokio::test(flavor = "multi_thread")]
async fn fichiers_dans_l_interface() {
    let ui = Arc::new(Window::default());
    let t = test_app_with("# conf\n", true, ui.clone()).await;
    let logs = t.dir.path().join("logs");
    std::fs::create_dir(&logs).unwrap();
    let app_log = logs.join("app.log");
    std::fs::write(&app_log, "2026-10-22 10:00:00,000 INFO un\n2026-10-22 10:00:01,000 ERROR deux\n").unwrap();
    std::fs::write(logs.join("batch-1.log"), "b1\n").unwrap();
    let path = app_log.to_string_lossy().to_string();
    let pattern = format!("{}/batch-*.log", logs.display());

    // Boîte d'ouverture.
    *ui.picked.lock() = vec![app_log.clone()];
    assert_eq!(c(&t, "files:pick", json!([])).await.unwrap(), json!([path]));

    // Récents, favoris, encodage.
    c(&t, "files:touch", json!([path])).await.unwrap();
    let list = c(&t, "files:touch", json!([pattern])).await.unwrap();
    assert_match(&list, &json!([{ "path": pattern, "pattern": true, "exists": true }, { "path": path, "name": "app.log", "pattern": false, "exists": true }]));
    let list = c(&t, "files:favorite", json!([path, true])).await.unwrap();
    assert_eq!(list[0]["favorite"], true);
    let list = c(&t, "files:setEncoding", json!([path, "latin-1"])).await.unwrap();
    assert_eq!(list[0]["encoding"], "latin-1");
    assert!(c(&t, "files:touch", json!(["relatif.log"])).await.is_err());
    // Rien dans config.yaml : propre au poste.
    assert!(!t.read("config.yaml").contains("app.log"));

    // Fichiers d'un motif dans l'arbre.
    let nodes = c(&t, "sources:browse", json!([{ "connectionId": "local", "path": [pattern] }])).await.unwrap();
    assert_match(&nodes, &json!({ "ok": true, "value": [{ "id": "batch-1.log", "kind": "file", "size": 3 }] }));

    // Flux : historique puis suivi, horodatage extrait de la ligne.
    let id = c(&t, "streams:open", json!([{ "connectionId": "local", "path": [path] }, { "follow": true, "tailLines": 10 }])).await.unwrap();
    let id = id.as_str().unwrap().to_string();
    wait_for(|| lines_of(&ui, &id).len() == 2).await;
    let first = ui.events.lock().iter().find(|e| e["event"]["type"] == "lines").cloned().unwrap();
    assert!(first["event"]["lines"][0]["ts"].as_str().unwrap().starts_with("2026-10-22T"), "{first}");
    std::fs::OpenOptions::new().append(true).open(&app_log).unwrap();
    use std::io::Write;
    writeln!(std::fs::OpenOptions::new().append(true).open(&app_log).unwrap(), "trois").unwrap();
    wait_for(|| lines_of(&ui, &id).len() == 3).await;
    c(&t, "streams:close", json!([id])).await.unwrap();

    // Fichier d'un motif ; nom hors motif refusé.
    let id = c(&t, "streams:open", json!([{ "connectionId": "local", "path": [pattern, "batch-1.log"] }, { "follow": false }])).await.unwrap();
    let id = id.as_str().unwrap().to_string();
    wait_for(|| lines_of(&ui, &id) == ["b1"]).await;
    assert!(c(&t, "streams:open", json!([{ "connectionId": "local", "path": [pattern, "../app.log"] }, { "follow": false }])).await.is_err());
    assert!(c(&t, "streams:open", json!([{ "connectionId": "local", "path": [path] }, { "follow": false, "encoding": "ebcdic" }])).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_limites_a_la_liste_blanche() {
    let t = test_app_with("# conf\n", true, Arc::new(crate::ui::NoUi)).await;
    let app = &t.app;
    let logs = t.dir.path().join("logs");
    std::fs::create_dir(&logs).unwrap();
    std::fs::write(logs.join("app-1.log"), "{\"level\":\"error\",\"message\":\"panne\"}\nok\n").unwrap();
    std::fs::write(t.dir.path().join("secret.txt"), "mot de passe\n").unwrap();
    let pattern = format!("{}/app-*.log", logs.display());
    let secret = t.dir.path().join("secret.txt").to_string_lossy().to_string();

    // Aucun fichier ouvert : outil masqué, lecture refusée.
    assert!(!names(app).await.contains(&"fichiers_lister".to_string()));
    let refused = call(app, "logs_lire", json!({ "connexion": "local", "chemin": [secret] })).await;
    assert!(refused.error && refused.text.contains("non autorisé"), "{}", refused.text);

    crate::settings::set_fichiers(&app.config, &json!({ "agents": [pattern] })).unwrap();
    assert!(t.read("config.yaml").contains("fichiers:"));
    assert!(crate::settings::set_fichiers(&app.config, &json!({ "agents": ["relatif/*.log"] })).is_err());
    assert!(names(app).await.contains(&"fichiers_lister".to_string()));
    let listed = call(app, "fichiers_lister", json!({})).await;
    assert_match(&listed.value, &json!([{ "connexion": "local", "chemin": [pattern], "motif": true, "present": true }]));
    let files = call(app, "sources_parcourir", json!({ "connexion": "local", "chemin": [pattern] })).await;
    assert_match(&files.value, &json!([{ "id": "app-1.log" }]));

    let read = call(app, "logs_lire", json!({ "connexion": "local", "chemin": [pattern], "niveaux": ["error"] })).await;
    assert!(!read.error, "{}", read.text);
    assert_match(&read.value, &json!({ "etat": "ended", "profil": "fichier/app.log", "recues": 2, "renvoyees": 1, "lignes": [{ "niveau": "error", "message": "panne" }] }));
    let one = call(app, "logs_lire", json!({ "connexion": "local", "chemin": [pattern, "app-1.log"] })).await;
    assert_eq!(one.value["recues"], 2);
    for chemin in [json!([secret]), json!([format!("{}/../secret.txt", logs.display())]), json!([format!("{}/*.txt", t.dir.path().display())])] {
        let r = call(app, "logs_lire", json!({ "connexion": "local", "chemin": chemin })).await;
        assert!(r.error, "{chemin} : {}", r.text);
    }
    let out = t.dir.path().join("export.log");
    let refused = call(app, "logs_exporter", json!({ "connexion": "local", "chemin": [secret], "fichier": out.to_string_lossy() })).await;
    assert!(refused.error && !out.exists());
}

#[test]
fn fichiers_de_la_ligne_de_commande() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.log"), "x\n").unwrap();
    let args = ["--data-dir=/tmp/x", "app.log", "absent.log", "."].map(String::from);
    let found = crate::files::paths_in_args(&args, dir.path());
    assert_eq!(found, vec![dir.path().join("app.log").to_string_lossy().into_owned()]);

    let files = crate::files::LocalFiles::new(dir.path().join("fichiers.json"));
    files.queue(found.clone());
    assert_eq!(files.take_pending(), found);
    assert!(files.take_pending().is_empty());
}
