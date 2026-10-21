//! Contexte IA : mémoire sur disque, découverte du LLM, configuration des agents, ingestion
//! Rocket.Chat et pipeline (curation par un faux LLM OpenAI-compatible).

use crate::config::Tls;
use crate::contexte::agents::{MCP_ENTRY, McpTarget, agent_states, configure_agent};
use crate::contexte::entity::{ContextEvent, event_from_item};
use crate::contexte::llm::{Curator, LlmConfig, discover};
use crate::contexte::state::RoomCursors;
use crate::contexte::store::{ContextStore, safe_name};
use crate::contexte::{ingest, rocketchat};
use crate::notifications::state::{EntityRef, NotifItem};
use crate::services::http::ServiceHttp;
use crate::testing::http::{Req, Resp, handler, start};
use serde_json::{Value, json};
use std::collections::HashMap;

fn event(over: impl FnOnce(&mut ContextEvent)) -> ContextEvent {
    let mut e = ContextEvent {
        entity: EntityRef { source: "gitlab".into(), r#type: "merge_request".into(), key: "grp/app!42".into(), title: "grp/app!42 « Paiement »".into(), url: Some("https://gitlab/grp/app/-/merge_requests/42".into()), participants: None },
        kind: "mr_comment".into(),
        at: "2026-09-29T12:00:00Z".into(),
        actor: Some("bob".into()),
        summary: "@bob a commenté votre MR".into(),
        body: Some("Le montant est appelé deux fois.".into()),
        url: Some("https://gitlab/grp/app/-/merge_requests/42#note_9".into()),
        dedup_key: "gitlab:note:9".into(),
    };
    over(&mut e);
    e
}

#[test]
fn memoire_de_contexte() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContextStore::new(dir.path());
    store.ingest(&event(|_| {})).unwrap();
    let file = dir.path().join("gitlab").join("grp-app!42.md");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("---\nsource: gitlab\n"), "{text}");
    assert!(text.contains("@bob a commenté votre MR") && text.contains("> Le montant est appelé deux fois.") && text.contains("[voir](https://gitlab"), "{text}");
    assert!(dir.path().join("AGENTS.md").exists());
    assert!(std::fs::read_to_string(dir.path().join("index.md")).unwrap().contains("grp/app!42"));

    // Cumul dans le même fichier, participants fusionnés.
    store.ingest(&event(|e| { e.dedup_key = "gitlab:note:10".into(); e.summary = "@alice a répondu".into(); e.body = Some("Le déploiement a échoué".into()); e.actor = Some("alice".into()); })).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("@bob a commenté") && text.contains("@alice a répondu"));
    assert!(text.contains("- bob") && text.contains("- alice"), "{text}");
    assert_eq!(store.size(), 1);

    // Recherche insensible à la casse et aux accents.
    assert_eq!(store.search("deploiement", 20).len(), 1);
    let hits = store.search("ECHOUE", 20);
    assert_eq!(hits[0].r#ref, "gitlab/grp-app!42.md");
    assert!(hits[0].extrait.contains("échoué"), "{}", hits[0].extrait);
    assert!(store.search("introuvable", 20).is_empty());
    assert!(store.search("  ", 20).is_empty());

    // Lecture par clé ou par référence.
    assert!(store.read("grp/app!42").unwrap().contenu.contains("Paiement"));
    assert!(store.read("gitlab/grp-app!42.md").unwrap().contenu.contains("Paiement"));
    assert!(store.read("inexistant").is_none());

    // Rechargement depuis le disque.
    let reloaded = ContextStore::new(dir.path());
    assert_eq!(reloaded.size(), 1);
    assert!(reloaded.recents(30)[0].titre.contains("Paiement"));
}

#[test]
fn repli_des_gros_fichiers_sans_perte() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContextStore::new(dir.path());
    let big = "x".repeat(20_000);
    for i in 0..20 { store.ingest(&event(|e| { e.dedup_key = format!("k{i}"); e.body = Some(big.clone()); })).unwrap(); }
    let archive = dir.path().join("gitlab").join("archive");
    let archived: Vec<_> = std::fs::read_dir(&archive).unwrap().collect();
    assert!(!archived.is_empty());
    let current = std::fs::read_to_string(dir.path().join("gitlab").join("grp-app!42.md")).unwrap();
    assert!(current.contains("archive") && current.len() < 256 * 1024);
    let total: usize = std::fs::read_dir(&archive).unwrap().map(|f| std::fs::read_to_string(f.unwrap().path()).unwrap().matches("## 2026").count()).sum::<usize>()
        + current.matches("## 2026").count();
    assert_eq!(total, 20, "aucun événement perdu");
}

#[test]
fn noms_et_evenements() {
    assert_eq!(safe_name("grp/app!42"), "grp-app!42");
    assert_eq!(safe_name("@amartin"), "@amartin");
    assert_eq!(safe_name("a<b>:c|d?"), "abcd");
    assert_eq!(safe_name("///"), "---");
    assert_eq!(safe_name("<>"), "sans-nom");
    let item = NotifItem {
        source: "jira".into(), kind: "comment".into(), title: "PAY-1".into(), actor: Some("bob".into()), dedup_key: "jira:PAY-1:c1".into(),
        entity: Some(EntityRef { source: "jira".into(), r#type: "ticket".into(), key: "PAY-1".into(), title: "PAY-1 « x »".into(), ..Default::default() }),
        body: Some("texte".into()), at: Some("2026-01-01T00:00:00Z".into()), ..Default::default()
    };
    let e = event_from_item(&item, "fallback").unwrap();
    assert_eq!((e.summary.as_str(), e.at.as_str()), ("@bob a commenté", "2026-01-01T00:00:00Z"));
    assert!(event_from_item(&NotifItem { source: "ocekour".into(), kind: "erreur".into(), title: "x".into(), dedup_key: "e".into(), ..Default::default() }, "now").is_none());
}

fn write(path: std::path::PathBuf, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn decouverte_du_llm() {
    let none = HashMap::new();
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(h.join(".qwen/settings.json"), r#"{ "model": { "name": "qwen2.5", "baseUrl": "http://localhost:11434" } }"#);
    let cfg = discover("auto", h, &none).unwrap();
    assert_eq!((cfg.source, cfg.model.as_str(), cfg.base_url.as_str()), ("qwen", "qwen2.5", "http://localhost:11434/v1"));

    write(h.join(".qwen/settings.json"), &json!({
        "env": { "VLLM_KEY": "sk-local" },
        "modelProviders": { "openai": [{ "id": "autre", "baseUrl": "http://x/v1", "envKey": "VLLM_KEY" }, { "id": "qwen3-coder", "baseUrl": "https://vllm.intra/v1", "envKey": "VLLM_KEY" }] },
        "security": { "auth": { "selectedType": "openai" } },
        "model": { "name": "qwen3-coder" },
    }).to_string());
    let cfg = discover("qwen", h, &none).unwrap();
    assert_eq!((cfg.model.as_str(), cfg.base_url.as_str(), cfg.api_key.as_deref()), ("qwen3-coder", "https://vllm.intra/v1", Some("sk-local")));

    write(h.join(".qwen/settings.json"), "{ // commentaire\n }");
    write(h.join(".qwen/.env"), "OPENAI_BASE_URL=https://llm.intra/v1\nOPENAI_MODEL=qwen2.5-coder\nOPENAI_API_KEY=\"sk-x\"\n");
    let cfg = discover("qwen", h, &none).unwrap();
    assert_eq!((cfg.base_url.as_str(), cfg.model.as_str(), cfg.api_key.as_deref()), ("https://llm.intra/v1", "qwen2.5-coder", Some("sk-x")));

    let other = tempfile::tempdir().unwrap();
    write(other.path().join(".vibe/config.toml"), "active_model = \"local\"\n\n[[models]]\nname = \"local\"\nmodel = \"devstral\"\nbase_url = \"http://127.0.0.1:8080\"\napi_key_env = \"MA_CLE\"\n");
    let env: HashMap<String, String> = [("MA_CLE".to_string(), "k".to_string())].into();
    let cfg = discover("auto", other.path(), &env).unwrap();
    assert_eq!((cfg.source, cfg.model.as_str(), cfg.base_url.as_str(), cfg.api_key.as_deref()), ("vibe", "devstral", "http://127.0.0.1:8080/v1", Some("k")));
    assert!(discover("off", h, &none).is_none());
}

#[test]
fn configuration_des_agents() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(h.join(".qwen/settings.json"), "{\"theme\": \"dark\"}");
    let target = McpTarget { url: "http://127.0.0.1:47110/mcp".into(), token: "T".into() };
    configure_agent("qwen", true, &target, h).unwrap();
    let json: Value = serde_json::from_str(&std::fs::read_to_string(h.join(".qwen/settings.json")).unwrap()).unwrap();
    assert_eq!(json["mcpServers"][MCP_ENTRY]["httpUrl"], "http://127.0.0.1:47110/mcp");
    assert_eq!(json["mcpServers"][MCP_ENTRY]["headers"]["Authorization"], "Bearer T");
    assert_eq!(json["theme"], "dark");
    assert!(agent_states(h).iter().find(|a| a.tool == "qwen").unwrap().configured);
    configure_agent("qwen", false, &target, h).unwrap();
    assert!(!agent_states(h).iter().find(|a| a.tool == "qwen").unwrap().configured);

    write(h.join(".vibe/config.toml"), "active_model = \"local\"\n");
    configure_agent("vibe", true, &target, h).unwrap();
    configure_agent("vibe", true, &target, h).unwrap();
    let toml = std::fs::read_to_string(h.join(".vibe/config.toml")).unwrap();
    assert_eq!(toml.matches("name = \"ocekour\"").count(), 1, "{toml}");
    assert_eq!(toml.matches("[mcp_servers.auth]").count(), 1, "{toml}");
    assert!(toml.contains("active_model = \"local\""));
    assert!(h.join(".vibe/config.toml.bak").exists());
    let vibe = agent_states(h).into_iter().find(|a| a.tool == "vibe").unwrap();
    assert!(vibe.present && vibe.configured);
    configure_agent("vibe", false, &target, h).unwrap();
    assert_eq!(std::fs::read_to_string(h.join(".vibe/config.toml")).unwrap(), "active_model = \"local\"\n");
}

async fn chat(routes: HashMap<&'static str, Value>) -> (crate::testing::http::TestServer, ServiceHttp) {
    let routes = std::sync::Arc::new(routes);
    let server = start(0, None, handler(move |r: Req| {
        let routes = routes.clone();
        async move {
            let path = r.path.trim_start_matches('/').to_string();
            match routes.iter().find(|(k, _)| **k == path) { Some((_, v)) => Resp::Json(200, v.clone()), None => Resp::Json(404, json!({})) }
        }
    })).await;
    let http = ServiceHttp { label: "Rocket.Chat".into(), url: server.url.clone(), tls: Tls::default(), auth: vec![], user_agent: "test".into() };
    (server, http)
}

#[tokio::test]
async fn ingestion_rocketchat() {
    let (_s, http) = chat([
        ("api/v1/me", json!({ "username": "moi" })),
        ("api/v1/subscriptions.get", json!({ "update": [{ "rid": "r1", "t": "d", "name": "bob", "unread": 2 }] })),
    ].into()).await;
    let (events, cursors) = rocketchat::fetch(&http, &RoomCursors::new(), "2026-09-29T12:00:00Z", true).await.unwrap();
    assert!(events.is_empty());
    assert_eq!(cursors["r1"], "2026-09-29T12:00:00Z");

    let (_s, http) = chat([
        ("api/v1/me", json!({ "username": "moi" })),
        ("api/v1/subscriptions.get", json!({ "update": [{ "rid": "r1", "t": "d", "name": "bob", "unread": 3 }, { "rid": "r2", "t": "c", "name": "general", "unread": 0, "lm": "2026-09-29T11:00:00Z" }] })),
        ("api/v1/im.history", json!({ "messages": [
            { "_id": "m1", "msg": "coucou", "u": { "username": "bob" }, "ts": "2026-09-29T12:30:00Z" },
            { "_id": "m2", "msg": "à moi", "u": { "username": "moi" }, "ts": "2026-09-29T12:31:00Z" },
            { "_id": "m3", "msg": "", "t": "uj", "u": { "username": "x" }, "ts": "2026-09-29T12:32:00Z" },
            { "_id": "m0", "msg": "ancien", "u": { "username": "bob" }, "ts": "2026-09-29T11:00:00Z" },
        ] })),
    ].into()).await;
    let before: RoomCursors = [("r1".to_string(), "2026-09-29T12:00:00Z".to_string()), ("r2".to_string(), "2026-09-29T12:00:00Z".to_string())].into();
    let (events, cursors) = rocketchat::fetch(&http, &before, "2026-09-29T13:00:00Z", false).await.unwrap();
    assert_eq!(events.len(), 2, "reçu + envoyé, ni système ni ancien");
    assert_eq!(events.iter().find(|e| e.actor.as_deref() == Some("moi")).unwrap().summary, "Vous avez écrit");
    let bob = events.iter().find(|e| e.actor.as_deref() == Some("bob")).unwrap();
    assert_eq!((bob.body.as_deref(), bob.entity.key.as_str()), (Some("coucou"), "@bob"));
    assert!(bob.url.as_deref().unwrap().ends_with("/direct/r1?msg=m1"));
    assert_eq!(cursors["r1"], "2026-09-29T13:00:00Z");
    assert_eq!(cursors["r2"], "2026-09-29T12:00:00Z", "salon calme : non inspecté");

    // Salon sans non-lu mais avec un message récent (envoyé).
    let (_s, http) = chat([
        ("api/v1/me", json!({ "username": "moi" })),
        ("api/v1/subscriptions.get", json!({ "update": [{ "rid": "r1", "t": "d", "name": "bob", "unread": 0, "lm": "2026-09-29T12:40:00Z" }] })),
        ("api/v1/im.history", json!({ "messages": [{ "_id": "m9", "msg": "ma décision", "u": { "username": "moi" }, "ts": "2026-09-29T12:40:00Z" }] })),
    ].into()).await;
    let (events, _) = rocketchat::fetch(&http, &[("r1".to_string(), "2026-09-29T12:00:00Z".to_string())].into(), "2026-09-29T13:00:00Z", false).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].summary.as_str(), events[0].body.as_deref()), ("Vous avez écrit", Some("ma décision")));
}

#[tokio::test]
async fn pipeline_avec_et_sans_llm() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContextStore::new(dir.path());
    let dm = |key: &str, body: &str, at: &str| ContextEvent {
        entity: EntityRef { source: "rocketchat".into(), r#type: "dm".into(), key: "@bob".into(), title: "@bob".into(), ..Default::default() },
        kind: "message".into(), at: at.into(), actor: Some("bob".into()), summary: "@bob a écrit".into(), body: Some(body.into()), url: None, dedup_key: key.into(),
    };
    let written = ingest(&store, &Curator::new(None, Tls::default()), vec![dm("rc:2", "deuxième", "2026-09-29T10:05:00Z"), dm("rc:1", "salut", "2026-09-29T10:00:00Z")]).await;
    assert_eq!(written, 2);
    let content = store.read("@bob").unwrap().contenu;
    assert!(content.find("salut").unwrap() < content.find("deuxième").unwrap(), "ordre chronologique");

    // Faux LLM : résume ; un second en erreur : texte brut gardé.
    let llm = start(0, None, handler(|r: Req| async move {
        let body = r.json();
        assert_eq!(r.header("authorization"), Some("Bearer sk"));
        let user = body["messages"][1]["content"].as_str().unwrap_or_default().to_string();
        if user.contains("panne") { return Resp::Json(500, json!({})); }
        Resp::Json(200, json!({ "choices": [{ "message": { "content": format!("Résumé : {}", user.lines().last().unwrap_or_default()) } }] }))
    })).await;
    let curator = Curator::new(Some(LlmConfig { base_url: format!("{}/v1", llm.url), model: "m".into(), api_key: Some("sk".into()), source: "qwen" }), Tls::default());
    assert_eq!(curator.describe().as_deref(), Some("qwen · m"));
    ingest(&store, &curator, vec![dm("rc:3", "un long message", "2026-09-29T11:00:00Z"), dm("rc:4", "panne totale", "2026-09-29T11:01:00Z")]).await;
    let content = store.read("@bob").unwrap().contenu;
    assert!(content.contains("> Résumé : un long message"), "{content}");
    assert!(content.contains("> panne totale"), "{content}");
}

struct ChatOnly(String);
impl crate::notifications::Clients for ChatOnly {
    fn available(&self, id: &str) -> bool { id == "rocketchat" }
    fn client(&self, _: &str) -> Result<ServiceHttp, crate::services::http::ServiceError> {
        Ok(ServiceHttp { label: "Rocket.Chat".into(), url: self.0.clone(), tls: Tls::default(), auth: vec![], user_agent: "test".into() })
    }
}
impl crate::contexte::Reachable for ChatOnly {
    fn reachable(&self, id: &str) -> bool { id == "rocketchat" }
}

#[tokio::test]
async fn moteur_depart_puis_ingestion() {
    let messages = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<Value>::new()));
    let m2 = messages.clone();
    let server = start(0, None, handler(move |r: Req| {
        let messages = m2.clone();
        async move {
            match r.path.as_str() {
                "/api/v1/me" => Resp::Json(200, json!({ "username": "moi" })),
                "/api/v1/subscriptions.get" => Resp::Json(200, json!({ "update": [{ "rid": "r1", "t": "d", "name": "bob", "unread": messages.lock().len() }] })),
                "/api/v1/im.history" => Resp::Json(200, json!({ "messages": *messages.lock() })),
                _ => Resp::Json(404, json!({})),
            }
        }
    })).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.yaml"), "contexte:\n  enabled: true\n  llm: off\n").unwrap();
    let clients = std::sync::Arc::new(ChatOnly(server.url.clone()));
    let store = std::sync::Arc::new(ContextStore::new(dir.path().join("contexte")));
    let tracked = std::sync::Arc::new(parking_lot::Mutex::new(vec![]));
    let t2 = tracked.clone();
    let engine = crate::contexte::ContextEngine::new(
        clients.clone(),
        clients,
        crate::config::ConfigStore::new(dir.path().join("config.yaml")),
        crate::contexte::state::ContextState::new(dir.path().join("contexte-state.json")),
        store.clone(),
        std::sync::Arc::new(move |e| t2.lock().push(e.value)),
        std::sync::Arc::new(|| true),
        std::sync::Arc::new(|_| None),
    );
    assert_eq!(engine.status().sources, vec!["rocketchat".to_string()]);
    engine.tick().await;
    assert_eq!(engine.status().entities, 0, "premier passage : aucun rattrapage");
    messages.lock().push(json!({ "_id": "m1", "msg": "nouveau", "u": { "username": "bob" }, "ts": crate::text::iso(chrono::Utc::now() + chrono::Duration::seconds(5)) }));
    engine.tick().await;
    let status = engine.status();
    assert_eq!((status.entities, status.last_count, status.last_error), (1, Some(1), None));
    assert!(store.read("@bob").unwrap().contenu.contains("> nouveau"));
    assert_eq!(*tracked.lock(), vec![Some(1.0)]);
    engine.tick().await;
    assert_eq!(engine.status().last_count, Some(0), "anti-doublon");
}
