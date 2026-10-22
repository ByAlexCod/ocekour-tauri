//! Tests d'intégration contre un vrai broker (scripts/kafka-test-broker.sh) :
//!   KAFKA_TEST_BROKER=localhost:19092 cargo test -p ocekour-core kafka
//! Broker < 3.1 (Fetch ≤ v12, topics par nom) : KAFKA_TEST_LEGACY_BROKER=localhost:19192.

use crate::config::{KafkaConnection, connection_schema, Connection};
use crate::error::ErrorCode;
use crate::kafka::client::ProduceMessage;
use crate::logs::LogLine;
use crate::streams::{StreamEvent, StreamOptions, StreamState, Timing};
use crate::testing::http::{self, Req, Resp};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

const SCHEMA: &str = r#"{"type":"record","name":"Commande","fields":[{"name":"id","type":"long"},{"name":"client","type":"string"},{"name":"note","type":["null","string"],"default":null}]}"#;

fn broker() -> Option<String> { std::env::var("KAFKA_TEST_BROKER").ok() }

fn connection(registry: &str, user: &str, extra: Value) -> KafkaConnection {
    let mut v = json!({
        "id": "kafka-test", "type": "kafka", "name": "test", "brokers": [broker().unwrap()],
        "security": { "username": user }, "registry": { "url": registry },
    });
    for (k, x) in extra.as_object().unwrap() { v[k] = x.clone(); }
    match connection_schema().parse_into::<Connection>(&v).unwrap() { Connection::Kafka(k) => k, _ => unreachable!() }
}

struct Fixture {
    run: String,
    t0: i64,
    registry: String,
    _server: http::TestServer,
}

static FIXTURE: OnceCell<Fixture> = OnceCell::const_new();

fn confluent(id: i32, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(&id.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

async fn fixture() -> &'static Fixture {
    FIXTURE
        .get_or_init(|| async {
            let server = http::start_shared(0, None, http::handler(|r: Req| async move {
                match r.path.as_str() {
                    "/subjects" => Resp::Json(200, json!(["commande-value"])),
                    "/schemas/ids/7" => Resp::Json(200, json!({ "schema": SCHEMA })),
                    _ => Resp::Json(404, json!({ "error_code": 40403, "message": "Schema not found" })),
                }
            }))
            .await;
            let run = format!("t{:x}", crate::text::now_ms());
            let t0 = crate::text::now_ms() - 60 * 60_000;
            let admin = crate::kafka::client(&connection(&server.url, "admin", json!({})), "admin-secret");
            let topic = format!("{run}.commandes");
            let avro_topic = format!("{run}.avro");
            admin.create_topics(&[&topic], 3, 1).await.unwrap();
            admin.create_topics(&[&avro_topic], 1, 1).await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
            let messages = (0..30)
                .map(|i| ProduceMessage {
                    partition: i % 3,
                    timestamp: Some(t0 + i as i64 * 60_000),
                    key: Some(format!("cmd-{i}").into_bytes()),
                    value: Some(json!({ "level": "INFO", "message": format!("commande {i}") }).to_string().into_bytes()),
                    headers: vec![("source".into(), b"test".to_vec())],
                })
                .collect();
            admin.produce(&topic, messages).await.unwrap();
            let schema = apache_avro::Schema::parse_str(SCHEMA).unwrap();
            let mut record = apache_avro::types::Record::new(&schema).unwrap();
            record.put("id", 42i64);
            record.put("client", "Dupont");
            record.put("note", apache_avro::types::Value::Union(0, Box::new(apache_avro::types::Value::Null)));
            #[allow(deprecated)]
            let avro = apache_avro::to_avro_datum(&schema, record).unwrap();
            let values: Vec<Vec<u8>> = vec![confluent(7, &avro), confluent(99, &[1, 2, 3]), b"texte simple".to_vec(), vec![0xff, 0x00, 0x10]];
            admin
                .produce(&avro_topic, values.into_iter().map(|v| ProduceMessage { value: Some(v), ..Default::default() }).collect())
                .await
                .unwrap();
            Fixture { run, t0, registry: server.url.clone(), _server: server }
        })
        .await
}

struct Out {
    lines: Vec<LogLine>,
    states: Vec<StreamState>,
    last: StreamEvent,
}

async fn read(conn: KafkaConnection, secret: &str, topic: &str, options: StreamOptions, during: Option<Box<dyn FnOnce(Arc<Mutex<Vec<StreamEvent>>>) -> tokio::task::JoinHandle<()> + Send>>, until: fn(&[StreamEvent]) -> bool) -> Out {
    let events = Arc::new(Mutex::new(Vec::<StreamEvent>::new()));
    let sink = events.clone();
    let cancel = tokio_util::sync::CancellationToken::new();
    let timing = Timing { batch: Duration::from_millis(10), batch_lines: 500, retry_delays: vec![Duration::from_millis(20), Duration::from_millis(50)], idle_timeout: Duration::from_secs(5) };
    let handle = crate::streams::kafka::spawn(conn, secret.into(), topic.into(), options, Arc::new(move |e| sink.lock().push(e)), timing, cancel.clone());
    if let Some(d) = during { d(events.clone()); }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline && !until(&events.lock()) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cancel.cancel();
    let _ = handle.await;
    let events = events.lock().clone();
    let statuses: Vec<StreamEvent> = events.iter().filter(|e| matches!(e, StreamEvent::Status { .. })).cloned().collect();
    Out {
        lines: events.iter().flat_map(|e| match e { StreamEvent::Lines { lines } => lines.clone(), _ => vec![] }).collect(),
        states: statuses.iter().map(|s| match s { StreamEvent::Status { state, .. } => *state, _ => unreachable!() }).collect(),
        last: statuses.last().cloned().unwrap(),
    }
}

fn ended(events: &[StreamEvent]) -> bool { events.iter().any(|e| matches!(e, StreamEvent::Status { state, .. } if state.is_final())) }
fn has_lines(events: &[StreamEvent]) -> bool { events.iter().any(|e| matches!(e, StreamEvent::Lines { .. })) }

fn opts(follow: bool, tail: Option<u64>, since: Option<u64>) -> StreamOptions {
    StreamOptions { follow, tail_lines: tail, since_seconds: since, previous: None, encoding: None }
}

#[tokio::test]
async fn kafka_test_de_connexion_et_topics() {
    if broker().is_none() { return; }
    let f = fixture().await;
    let summary = crate::kafka::test(&connection(&f.registry, "lecteur", json!({})), "lecteur-secret").await.unwrap();
    assert!(regex::Regex::new(r"^1 broker\(s\), \d+ topic\(s\), Schema Registry OK \(1 sujet\(s\)\)$").unwrap().is_match(&summary), "{summary}");
    let e = crate::kafka::test(&connection(&f.registry, "lecteur", json!({})), "faux").await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Unauthorized);
    let topics = crate::kafka::list_topics(&connection(&f.registry, "lecteur", json!({})), "lecteur-secret").await.unwrap();
    let t = topics.iter().find(|t| t.id == format!("{}.commandes", f.run)).unwrap();
    assert_eq!((t.kind, t.partitions), ("topic", Some(3)));
    assert!(!topics.iter().any(|t| t.id.starts_with("__")));
}

#[tokio::test]
async fn kafka_n_derniers_fusionnes_par_date() {
    if broker().is_none() { return; }
    let f = fixture().await;
    let out = read(connection(&f.registry, "lecteur", json!({})), "lecteur-secret", &format!("{}.commandes", f.run), opts(false, Some(10), None), None, ended).await;
    assert_eq!(out.states, vec![StreamState::Connecting, StreamState::Live, StreamState::Ended]);
    let keys: Vec<String> = out.lines.iter().map(|l| l.meta.as_ref().unwrap().key.clone().unwrap()).collect();
    assert_eq!(keys, (20..30).map(|i| format!("cmd-{i}")).collect::<Vec<_>>());
    let first = &out.lines[0];
    assert_eq!(first.ts.as_deref(), Some(crate::text::iso(chrono::DateTime::from_timestamp_millis(f.t0 + 20 * 60_000).unwrap()).as_str()));
    assert_eq!(first.text, json!({ "level": "INFO", "message": "commande 20" }).to_string());
    let meta = first.meta.as_ref().unwrap();
    assert_eq!((meta.partition, meta.offset.as_str(), meta.format.as_str()), (2, "6", "json"));
    assert_eq!(meta.headers.as_ref().unwrap().get("source").map(String::as_str), Some("test"));
}

#[tokio::test]
async fn kafka_depuis_et_tout() {
    if broker().is_none() { return; }
    let f = fixture().await;
    let topic = format!("{}.commandes", f.run);
    let since = ((crate::text::now_ms() - (f.t0 + 25 * 60_000)) as f64 / 1000.0).ceil() as u64 + 1;
    let out = read(connection(&f.registry, "lecteur", json!({})), "lecteur-secret", &topic, opts(false, None, Some(since)), None, ended).await;
    let mut keys: Vec<String> = out.lines.iter().map(|l| l.meta.as_ref().unwrap().key.clone().unwrap()).collect();
    keys.sort();
    assert_eq!(keys, vec!["cmd-25", "cmd-26", "cmd-27", "cmd-28", "cmd-29"]);
    let out = read(connection(&f.registry, "lecteur", json!({})), "lecteur-secret", &topic, opts(false, None, None), None, ended).await;
    assert_eq!(out.lines.len(), 30);
}

#[tokio::test]
async fn kafka_suivi_en_direct() {
    if broker().is_none() { return; }
    let f = fixture().await;
    let topic = format!("{}.commandes", f.run);
    let registry = f.registry.clone();
    let t = topic.clone();
    let during: Box<dyn FnOnce(Arc<Mutex<Vec<StreamEvent>>>) -> tokio::task::JoinHandle<()> + Send> = Box::new(move |events| {
        tokio::spawn(async move {
            while !events.lock().iter().any(|e| matches!(e, StreamEvent::Status { state: StreamState::Live, .. })) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            let admin = crate::kafka::client(&connection(&registry, "admin", json!({})), "admin-secret");
            admin.produce(&t, vec![ProduceMessage { partition: 1, value: Some(b"en direct".to_vec()), ..Default::default() }]).await.unwrap();
        })
    });
    let out = read(connection(&f.registry, "lecteur", json!({})), "lecteur-secret", &topic, opts(true, Some(0), None), Some(during), has_lines).await;
    assert_eq!(out.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["en direct"]);
}

#[tokio::test]
async fn kafka_avro_registre_texte_binaire() {
    if broker().is_none() { return; }
    let f = fixture().await;
    let out = read(connection(&f.registry, "lecteur", json!({})), "lecteur-secret", &format!("{}.avro", f.run), opts(false, None, None), None, ended).await;
    let got: Vec<(String, String, String)> = out
        .lines
        .iter()
        .map(|l| { let m = l.meta.as_ref().unwrap(); (m.format.clone(), l.text.clone(), m.warning.clone().unwrap_or_default()) })
        .collect();
    assert_eq!(got[0], ("avro".into(), r#"{"id":42,"client":"Dupont","note":null}"#.into(), String::new()));
    assert_eq!((got[1].0.as_str(), got[1].1.as_str()), ("hex", "0000000063010203"));
    assert!(got[1].2.contains("schéma 99"), "{}", got[1].2);
    assert_eq!(got[2], ("text".into(), "texte simple".into(), String::new()));
    assert_eq!(got[3], ("hex".into(), "ff0010".into(), String::new()));
}

#[tokio::test]
async fn kafka_topic_inexistant_et_aucun_groupe() {
    if broker().is_none() { return; }
    let f = fixture().await;
    let out = read(connection(&f.registry, "lecteur", json!({})), "lecteur-secret", &format!("{}.absent", f.run), opts(false, None, None), None, ended).await;
    assert!(matches!(out.last, StreamEvent::Status { state: StreamState::Error, .. }));
    let admin = crate::kafka::client(&connection(&f.registry, "admin", json!({})), "admin-secret");
    assert!(admin.list_groups().await.unwrap().is_empty());
}

#[tokio::test]
async fn kafka_broker_ancien_sans_ids_de_topics() {
    let Ok(legacy) = std::env::var("KAFKA_TEST_LEGACY_BROKER") else { return };
    let conn = match connection_schema()
        .parse_into::<Connection>(&json!({ "id": "kafka-ancien", "type": "kafka", "name": "ancien", "brokers": [legacy], "security": { "protocol": "PLAINTEXT" } }))
        .unwrap()
    {
        Connection::Kafka(k) => k,
        _ => unreachable!(),
    };
    let topic = format!("t{:x}.ancien", crate::text::now_ms());
    let c = crate::kafka::client(&conn, "");
    c.create_topics(&[&topic], 2, 1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let t0 = crate::text::now_ms() - 60_000;
    c.produce(&topic, (0..6).map(|i| ProduceMessage { partition: i % 2, timestamp: Some(t0 + i as i64 * 1000), value: Some(format!("message {i}").into_bytes()), ..Default::default() }).collect())
        .await
        .unwrap();
    let out = read(conn, "", &topic, opts(false, Some(4), None), None, ended).await;
    assert!(matches!(out.last, StreamEvent::Status { state: StreamState::Ended, .. }));
    let mut texts: Vec<String> = out.lines.iter().map(|l| l.text.clone()).collect();
    texts.sort();
    assert_eq!(texts, vec!["message 2", "message 3", "message 4", "message 5"]);
}

/// « Télécharger… » sur un topic : tous les messages, dans l'ordre chronologique (toutes
/// partitions), format NDJSON avec métadonnées, filtre de recherche.
#[tokio::test(flavor = "multi_thread")]
async fn telechargement_d_un_topic() {
    if broker().is_none() { return; }
    let f = fixture().await;
    #[derive(Default)]
    struct SaveUi(Mutex<Vec<std::path::PathBuf>>);
    #[async_trait::async_trait]
    impl crate::ui::Ui for SaveUi {
        async fn save_dialog(&self, _: &str, _: std::path::PathBuf, _: Vec<crate::ui::FileFilter>) -> Option<std::path::PathBuf> { self.0.lock().pop() }
    }
    let ui = Arc::new(SaveUi::default());
    let t = super::support::test_app_with("# conf\n", true, ui.clone()).await;
    let id = t.app.connections.save(&json!({ "type": "kafka", "name": "test", "brokers": [broker().unwrap()], "security": { "username": "lecteur" }, "secret": "lecteur-secret" })).unwrap();
    let caller = crate::api::Caller { window: "main".into() };
    let topic = format!("{}.commandes", f.run);
    let download = |name: &str, request: Value| {
        let out = t.path(name);
        ui.0.lock().push(out.clone());
        let (app, caller) = (t.app.clone(), caller.clone());
        async move {
            let r = tokio::time::timeout(Duration::from_secs(60), crate::api::call(&app, &caller, "export:downloadLogs", vec![request])).await.expect("téléchargement bloqué");
            (r, std::fs::read_to_string(&out).unwrap_or_default())
        }
    };
    let (r, written) = download("topic.ndjson", json!({ "defaultName": "x", "filtered": false, "sources": [{ "connectionId": id, "path": [topic] }] })).await;
    assert_eq!(r.unwrap()["lines"], 30);
    let lines: Vec<Value> = written.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let order: Vec<String> = lines.iter().map(|l| l["value"]["message"].as_str().unwrap().to_string()).collect();
    assert_eq!(order, (0..30).map(|i| format!("commande {i}")).collect::<Vec<_>>(), "ordre chronologique, toutes partitions");
    assert_eq!((lines[0]["key"].as_str(), lines[0]["headers"]["source"].as_str()), (Some("cmd-0"), Some("test")));
    let request = json!({ "defaultName": "x", "filtered": true, "search": { "text": "commande 2", "caseSensitive": false, "regex": false }, "sources": [{ "connectionId": id, "path": [topic] }] });
    let (r, written) = download("filtre.ndjson", request).await;
    assert_eq!(r.unwrap()["lines"], 11);
    assert_eq!(written.lines().count(), 11);
    let (r, _) = download("absent.ndjson", json!({ "defaultName": "x", "filtered": false, "sources": [{ "connectionId": id, "path": ["topic-inexistant-zz"] }] })).await;
    assert!(r.is_err(), "topic inexistant : {r:?}");
}
