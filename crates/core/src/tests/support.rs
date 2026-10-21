//! Application de test (dossier temporaire, licence activée) et client MCP en mémoire.

use crate::app::{App, AppOptions};
use crate::licence::LicenceKeys;
use crate::licence::format::{self, Keyring};
use crate::mcp::Session;
use crate::ui::NoUi;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct TestApp {
    pub app: Arc<App>,
    pub dir: tempfile::TempDir,
    /// Clé d'activation valide pour cette application.
    pub key: String,
}

impl TestApp {
    pub fn path(&self, name: &str) -> std::path::PathBuf { self.dir.path().join(name) }
    pub fn read(&self, name: &str) -> String { std::fs::read_to_string(self.path(name)).unwrap_or_default() }
}

/// Application dans un dossier temporaire ; `config` : contenu initial de config.yaml.
pub async fn test_app(config: &str, activate: bool) -> TestApp { test_app_with(config, activate, Arc::new(NoUi)).await }

pub async fn test_app_with(config: &str, activate: bool, ui: Arc<dyn crate::ui::Ui>) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.yaml"), config).unwrap();
    let (private, public) = format::generate_signing_keys();
    let obfuscation = format::generate_obfuscation_key();
    let keys = LicenceKeys {
        keyring: Keyring { public_keys: vec![("prod1".into(), public)], obfuscation_keys: vec![obfuscation.clone()] },
        revoked: vec![],
        development: true,
    };
    let payload = json!({ "v": 1, "id": format::new_licence_id(), "label": "Tests", "issuedAt": "2026-09-24", "mode": "none" });
    let key = format::encode(&payload, "prod1", &private, &obfuscation).unwrap();
    let mut options = AppOptions::new(dir.path(), "0.3.0", ui);
    options.licence_keys = Some(keys);
    let app = App::new(options).unwrap();
    if activate { app.activate(&key).await.unwrap(); }
    TestApp { app, dir, key }
}

/// Résultat d'un appel d'outil : JSON si la réponse en est, sinon texte.
#[derive(Debug, Clone)]
pub struct Called {
    pub value: Value,
    pub text: String,
    pub error: bool,
}

/// Session initialisée par un client « test-agent 9.9 ».
pub async fn session(app: &Arc<App>) -> Session {
    let mut s = Session::new(app.clone());
    s.handle(json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test-agent", "version": "9.9" },
    }})).await;
    s
}

pub async fn names(app: &Arc<App>) -> Vec<String> {
    let r = session(app).await.handle(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await.unwrap();
    r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

pub async fn call(app: &Arc<App>, name: &str, args: Value) -> Called {
    let r = session(app).await.handle(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": name, "arguments": args } })).await.unwrap();
    let result = &r["result"];
    assert!(result.is_object(), "{r}");
    let text = result["content"][0]["text"].as_str().unwrap_or_default().to_string();
    Called { value: serde_json::from_str(&text).unwrap_or(Value::String(text.clone())), text, error: result["isError"] == true }
}

/// Vérifie que `actual` contient au moins les champs de `expected` (récursivement).
#[track_caller]
pub fn assert_match(actual: &Value, expected: &Value) {
    fn matches(a: &Value, e: &Value) -> bool {
        match (a, e) {
            (Value::Object(a), Value::Object(e)) => e.iter().all(|(k, v)| a.get(k).is_some_and(|av| matches(av, v))),
            (Value::Array(a), Value::Array(e)) => a.len() == e.len() && a.iter().zip(e).all(|(x, y)| matches(x, y)),
            _ => a == e,
        }
    }
    assert!(matches(actual, expected), "\nobtenu : {}\nattendu : {}", serde_json::to_string_pretty(actual).unwrap(), serde_json::to_string_pretty(expected).unwrap());
}
