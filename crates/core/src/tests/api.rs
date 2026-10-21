//! Canaux de l'interface : garde de licence, configuration, connexions, flux (événements vers
//! la fenêtre), exports via boîtes de dialogue, téléchargement d'historique, comptes, MCP.

use super::support::{TestApp, test_app_with};
use crate::api::{Caller, call};
use crate::testing::fake_rancher::{self, FAKE_TOKEN, HEALTHY};
use crate::ui::{FileFilter, OpenOptions, Ui};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Interface de test : boîtes de dialogue scriptées, événements et presse-papiers enregistrés.
#[derive(Default)]
struct RecordingUi {
    events: Mutex<Vec<(String, String, Value)>>,
    save_answers: Mutex<Vec<Option<PathBuf>>>,
    open_answers: Mutex<Vec<Vec<PathBuf>>>,
    clipboard: Mutex<String>,
    opened: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Ui for RecordingUi {
    fn emit(&self, event: &str, payload: Value) { self.events.lock().push(("*".into(), event.into(), payload)); }
    fn emit_to(&self, window: &str, event: &str, payload: Value) { self.events.lock().push((window.into(), event.into(), payload)); }
    async fn save_dialog(&self, _: &str, _: PathBuf, _: Vec<FileFilter>) -> Option<PathBuf> { self.save_answers.lock().pop().flatten() }
    async fn open_dialog(&self, _: OpenOptions) -> Vec<PathBuf> { self.open_answers.lock().pop().unwrap_or_default() }
    fn copy_text(&self, text: &str) -> anyhow::Result<()> { *self.clipboard.lock() = text.into(); Ok(()) }
    fn read_clipboard(&self) -> anyhow::Result<String> { Ok(self.clipboard.lock().clone()) }
    fn open_url(&self, url: &str) -> anyhow::Result<()> { self.opened.lock().push(url.into()); Ok(()) }
}

impl RecordingUi {
    fn of(&self, event: &str) -> Vec<(String, Value)> { self.events.lock().iter().filter(|(_, e, _)| e == event).map(|(w, _, p)| (w.clone(), p.clone())).collect() }
}

async fn setup(activate: bool) -> (TestApp, Arc<RecordingUi>) {
    let ui = Arc::new(RecordingUi::default());
    let t = test_app_with("# conf\n", activate, ui.clone()).await;
    (t, ui)
}

fn win() -> Caller { Caller { window: "main".into() } }

async fn c(t: &TestApp, channel: &str, args: Value) -> Result<Value, String> {
    call(&t.app, &win(), channel, args.as_array().cloned().unwrap_or_default()).await
}

#[tokio::test(flavor = "multi_thread")]
async fn garde_de_licence() {
    let (t, ui) = setup(false).await;
    assert_eq!(c(&t, "connections:hasSecret", json!(["x"])).await.unwrap_err(), "Ocekour n'est pas activé.");
    assert!(c(&t, "config:get", json!([])).await.unwrap()["config"].is_object());
    assert_eq!(c(&t, "licence:state", json!([])).await.unwrap()["status"], "missing");
    c(&t, "app:copyText", json!(["bonjour"])).await.unwrap();
    assert_eq!(c(&t, "app:readClipboard", json!([])).await.unwrap(), "bonjour");
    let bad = c(&t, "licence:activate", json!(["n'importe quoi"])).await.unwrap();
    assert_eq!((bad["ok"].as_bool(), bad["error"]["code"].as_str()), (Some(false), Some("invalid")));
    let ok = c(&t, "licence:activate", json!([t.key])).await.unwrap();
    assert_eq!((ok["ok"].as_bool(), ok["value"]["status"].as_str()), (Some(true), Some("active")));
    assert!(ui.of("licence:changed").iter().any(|(_, s)| s["status"] == "active"));
    assert_eq!(c(&t, "connections:hasSecret", json!(["x"])).await.unwrap(), json!(false));
    assert!(c(&t, "canal:inconnu", json!([])).await.unwrap_err().contains("inconnu"));
}

#[tokio::test(flavor = "multi_thread")]
async fn configuration_et_preferences() {
    let (t, ui) = setup(true).await;
    let state = c(&t, "config:setPreferences", json!([{ "theme": "dark" }])).await.unwrap();
    assert_eq!(state["config"]["theme"], "dark");
    assert!(t.read("config.yaml").starts_with("# conf\n"));
    assert!(c(&t, "config:setPreferences", json!([{ "inconnue": 1 }])).await.unwrap_err().contains("Préférence inconnue"));
    assert!(ui.of("config:changed").iter().any(|(_, s)| s["config"]["theme"] == "dark"));
    let errors = c(&t, "config:validateText", json!(["mcpPort: pas-un-port\n"])).await.unwrap();
    assert!(!errors.as_array().unwrap().is_empty());
    assert_eq!(c(&t, "config:validateText", json!(["theme: light\n"])).await.unwrap(), json!([]));
    let saved = c(&t, "config:saveText", json!(["theme: light\n"])).await.unwrap();
    assert_eq!(saved["ok"], true);
    assert_eq!(c(&t, "config:readText", json!([])).await.unwrap(), "theme: light\n");
    let fav = c(&t, "config:toggleFavorite", json!(["c/prod/app"])).await.unwrap();
    assert_eq!(fav["config"]["favorites"], json!(["c/prod/app"]));
    let profile = c(&t, "config:editProfile", json!(["prod/app/api", { "op": "toggleColumn", "path": "user" }])).await.unwrap();
    assert_eq!(profile["config"]["profiles"][0]["columns"], json!(["user"]));
    c(&t, "config:saveFilterPreset", json!(["erreurs", [{ "field": "level", "value": "error", "action": "only" }]])).await.unwrap();
    c(&t, "config:renameFilterPreset", json!(["erreurs", "graves"])).await.unwrap();
    let presets = c(&t, "config:deleteFilterPreset", json!(["graves"])).await.unwrap();
    assert_eq!(presets["config"]["filterPresets"], json!([]));
    let notif = c(&t, "config:setNotifications", json!([{ "intervalSeconds": 120, "sources": { "jira": { "enabled": true } } }])).await.unwrap();
    assert_eq!((notif["config"]["notifications"]["intervalSeconds"].as_u64(), notif["config"]["notifications"]["sources"]["jira"]["enabled"].as_bool()), (Some(120), Some(true)));
    let ctx = c(&t, "config:setContexte", json!([{ "llm": "off" }])).await.unwrap();
    assert_eq!(ctx["config"]["contexte"]["llm"], "off");
    c(&t, "app:logError", json!(["erreur de l'interface"])).await.unwrap();
    let mcp = c(&t, "mcp:status", json!([])).await.unwrap();
    let renewed = c(&t, "mcp:regenerateToken", json!([])).await.unwrap();
    assert_ne!(mcp["token"], renewed["token"]);
    assert_eq!(renewed["url"], "http://127.0.0.1:47110/mcp");
    assert_eq!(c(&t, "analytics:status", json!([])).await.unwrap(), json!({ "enabled": false }));
    c(&t, "analytics:event", json!([{ "category": "Écran", "action": "ouvert" }])).await.unwrap();
    assert!(c(&t, "analytics:event", json!([{ "category": "" }])).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn connexions_sources_flux_et_exports() {
    let rancher = fake_rancher::start(0).await;
    rancher.control.interval_ms.store(20, Ordering::SeqCst);
    let (t, ui) = setup(true).await;
    let draft = json!({ "type": "rancher", "name": "Rancher test", "url": rancher.url, "secret": FAKE_TOKEN, "tls": { "insecure": true }, "clusters": [{ "id": "c-prod1", "name": "prod" }] });
    let tested = c(&t, "connections:test", json!([draft])).await.unwrap();
    assert_eq!(tested, json!({ "ok": true, "value": { "summary": "Connecté en tant que jdupont" } }));
    let clusters = c(&t, "connections:listClusters", json!([draft])).await.unwrap();
    assert_eq!(clusters["ok"], true);
    let id = c(&t, "connections:save", json!([draft])).await.unwrap();
    let id = id.as_str().unwrap().to_string();
    assert_eq!(c(&t, "connections:hasSecret", json!([id])).await.unwrap(), json!(true));
    let refused = c(&t, "connections:test", json!([{ "type": "rancher", "name": "x", "url": rancher.url, "secret": "mauvais", "tls": { "insecure": true }, "clusters": [] }])).await.unwrap();
    assert_eq!((refused["ok"].as_bool(), refused["error"]["code"].as_str()), (Some(false), Some("unauthorized")));

    let pods = c(&t, "sources:browse", json!([{ "connectionId": id, "path": ["c-prod1", "app-commandes"] }])).await.unwrap();
    assert!(pods["value"].as_array().unwrap().iter().any(|p| p["id"] == HEALTHY));

    // Flux : événements adressés à la fenêtre qui l'a ouvert.
    let path = json!(["c-prod1", "app-commandes", HEALTHY, "app"]);
    let stream = c(&t, "streams:open", json!([{ "connectionId": id, "path": path }, { "follow": false, "tailLines": 5 }])).await.unwrap();
    let stream = stream.as_str().unwrap().to_string();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ui.of("streams:event").iter().any(|(_, p)| p["event"]["state"] == "ended") { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.unwrap();
    let events = ui.of("streams:event");
    assert!(events.iter().all(|(w, p)| w == "main" && p["id"] == stream.as_str()));
    let lines: usize = events.iter().filter(|(_, p)| p["event"]["type"] == "lines").map(|(_, p)| p["event"]["lines"].as_array().unwrap().len()).sum();
    assert_eq!(lines, 5);
    c(&t, "streams:close", json!([stream])).await.unwrap();
    assert!(c(&t, "streams:open", json!([{ "connectionId": id, "path": path }, { "follow": "oui" }])).await.is_err());

    // Export : seul le fichier choisi dans la boîte peut être écrit, une fois.
    ui.save_answers.lock().push(None);
    assert_eq!(c(&t, "export:pick", json!(["logs"])).await.unwrap(), Value::Null);
    let file = t.path("export.log");
    ui.save_answers.lock().push(Some(file.clone()));
    let picked = c(&t, "export:pick", json!(["logs"])).await.unwrap();
    assert_eq!(picked["format"], "text");
    let token = picked["token"].as_str().unwrap().to_string();
    c(&t, "export:write", json!([token, "a\nb\n"])).await.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "a\nb\n");
    assert!(c(&t, "export:write", json!([token, "x"])).await.unwrap_err().contains("expiré"));

    // Historique complet : deux sources assemblées, filtre de niveau.
    let out = t.path("historique.ndjson");
    ui.save_answers.lock().push(Some(out.clone()));
    let request = json!({ "defaultName": "commandes", "filtered": false, "sources": [
        { "connectionId": id, "path": path, "label": "api" },
        { "connectionId": id, "path": ["c-prod1", "app-commandes", HEALTHY, "app"], "label": "api-bis", "includePrevious": true },
    ] });
    let result = c(&t, "export:downloadLogs", json!([request])).await.unwrap();
    let written = std::fs::read_to_string(&out).unwrap();
    assert_eq!(result["lines"].as_u64().unwrap() as usize, written.lines().count());
    assert!(result["lines"].as_u64().unwrap() > 0 && result["truncated"] == false);
    assert!(written.lines().all(|l| l.contains("\"source\":\"api")), "{}", written.lines().next().unwrap());
    ui.save_answers.lock().push(None);
    assert_eq!(c(&t, "export:downloadLogs", json!([request])).await.unwrap(), Value::Null);

    // Suppression.
    c(&t, "connections:delete", json!([id])).await.unwrap();
    assert_eq!(c(&t, "connections:hasSecret", json!([id])).await.unwrap(), json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn comptes_journal_et_contexte() {
    let (t, ui) = setup(true).await;
    let accounts = c(&t, "services:list", json!([])).await.unwrap();
    assert_eq!(accounts.as_array().unwrap().len(), 4);
    let saved = c(&t, "services:save", json!(["jira", { "url": "https://jira.intra", "tls": { "insecure": false } }])).await.unwrap();
    assert!(saved.as_array().unwrap().iter().any(|a| a["id"] == "jira" && a["url"] == "https://jira.intra"));
    assert!(c(&t, "services:save", json!(["github", { "url": "x", "tls": { "insecure": false } }])).await.unwrap_err().contains("inconnu"));
    c(&t, "services:openTokenPage", json!(["jira"])).await.unwrap();
    assert!(ui.opened.lock()[0].starts_with("https://jira.intra/"));
    let flags = c(&t, "services:setFlags", json!(["jira", { "write": true }])).await.unwrap();
    assert!(flags.as_array().unwrap().iter().any(|a| a["id"] == "jira" && a["write"] == true));
    assert_eq!(c(&t, "actions:list", json!([{}])).await.unwrap(), json!({ "entries": [], "total": 0 }));
    assert!(c(&t, "actions:openUrl", json!(["javascript:alert(1)"])).await.is_err());
    c(&t, "actions:openUrl", json!(["https://gitlab.intra/x"])).await.unwrap();
    assert_eq!(c(&t, "notifications:status", json!([])).await.unwrap()["enabled"], false);
    let test = c(&t, "notifications:test", json!([])).await.unwrap();
    assert_eq!(test["ok"], false);
    assert_eq!(c(&t, "contexte:status", json!([])).await.unwrap()["entities"], 0);
    assert_eq!(c(&t, "databases:list", json!([])).await.unwrap(), json!([]));
    assert!(c(&t, "contexte:configureAgent", json!(["autre", "add"])).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn notifications_publiees_a_l_interface() {
    let (t, ui) = setup(true).await;
    t.app.start_background();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(ui.of("notifications:changed").last().is_some_and(|(_, s)| s["enabled"] == false));
    c(&t, "config:setNotifications", json!([{ "enabled": true }])).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ui.of("notifications:changed").iter().any(|(_, s)| s["enabled"] == true && s["running"] == true) { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.unwrap();
    c(&t, "config:setContexte", json!([{ "enabled": true }])).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ui.of("contexte:changed").iter().any(|(_, s)| s["running"] == true) { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.unwrap();
    t.app.shutdown().await;
}
