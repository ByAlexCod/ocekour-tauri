//! Faux Rancher 2.5 en HTTPS (certificat auto-signé) : tests et démonstrations sans vrai
//! serveur (`ocekour-headless fake-rancher`). API Kubernetes simulée derrière le proxy
//! (`/k8s/clusters/<id>/api/v1/…`) et logs alimentés en continu.

use super::http::{self, Req, Resp, TestServer};
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::mpsc;

pub const FAKE_TOKEN: &str = "token-fake:secret";
pub const CRASHING: &str = "api-commandes-7d9f8b6c5d-q8w7e";
pub const HEALTHY: &str = "api-commandes-7d9f8b6c5d-x2k4p";

/// Pilotage du faux serveur.
pub struct Control {
    /// Intervalle entre deux lignes en direct (ms).
    pub interval_ms: AtomicU64,
    /// Coupe les flux suivis après ce délai (ms, 0 = jamais) : coupure réseau simulée.
    pub drop_after_ms: AtomicU64,
    /// Pods supprimés (404).
    pub deleted: Mutex<HashSet<String>>,
    /// Lignes d'historique par conteneur.
    pub history: AtomicUsize,
    /// Lignes émises à chaque intervalle.
    pub burst: AtomicUsize,
}

impl Default for Control {
    fn default() -> Self {
        Control {
            interval_ms: AtomicU64::new(300),
            drop_after_ms: AtomicU64::new(0),
            deleted: Mutex::new(HashSet::new()),
            history: AtomicUsize::new(2_000),
            burst: AtomicUsize::new(1),
        }
    }
}

#[derive(Clone)]
struct Entry {
    ts: String,
    text: String,
}

type Listener = mpsc::UnboundedSender<Entry>;

#[derive(Default)]
struct Logs {
    stores: HashMap<String, Vec<Entry>>,
    listeners: HashMap<String, Vec<Listener>>,
    ticker: bool,
    seq: usize,
}

pub struct FakeRancher {
    pub url: String,
    /// Certificat du serveur (auto-signé, sert aussi de CA), PEM.
    pub ca_pem: String,
    pub ca_der: Vec<u8>,
    pub control: Arc<Control>,
    _server: TestServer,
}

pub fn fake_clusters() -> Value {
    json!([
        { "id": "c-prod1", "name": "prod", "state": "active" },
        { "id": "c-recet", "name": "recette", "state": "active" },
        { "id": "local", "name": "local", "state": "active" },
    ])
}

fn ago(minutes: i64) -> String { crate::text::iso(chrono::Utc::now() - chrono::Duration::minutes(minutes)) }

fn running(name: &str, restarts: u64, last_terminated: bool) -> Value {
    json!({
        "name": name, "ready": true, "restartCount": restarts,
        "state": { "running": { "startedAt": ago(30) } },
        "lastState": if last_terminated { json!({ "terminated": { "exitCode": 1, "reason": "Error" } }) } else { json!({}) },
    })
}

fn deployment_pod(name: &str, hash: &str, suffix: &str) -> Value {
    json!({
        "metadata": {
            "name": format!("{name}-{hash}-{suffix}"), "namespace": "app-commandes", "creationTimestamp": ago(180),
            "labels": { "app": name, "pod-template-hash": hash },
            "ownerReferences": [{ "kind": "ReplicaSet", "name": format!("{name}-{hash}"), "controller": true }],
        },
        "spec": { "containers": [{ "name": "app" }, { "name": "istio-proxy" }] },
        "status": { "phase": "Running", "containerStatuses": [running("app", 0, false), running("istio-proxy", 0, false)] },
    })
}

/// Pods simulés par namespace.
pub fn fake_pods() -> Vec<(&'static str, Vec<Value>)> {
    let mut crashing = deployment_pod("api-commandes", "7d9f8b6c5d", "q8w7e");
    crashing["spec"] = json!({ "initContainers": [{ "name": "migrations" }], "containers": [{ "name": "app" }, { "name": "istio-proxy" }] });
    crashing["status"] = json!({
        "phase": "Running",
        "initContainerStatuses": [{ "name": "migrations", "ready": true, "restartCount": 0, "state": { "terminated": { "exitCode": 0, "reason": "Completed" } } }],
        "containerStatuses": [
            { "name": "app", "ready": false, "restartCount": 5, "state": { "waiting": { "reason": "CrashLoopBackOff" } }, "lastState": { "terminated": { "exitCode": 1, "reason": "Error" } } },
            running("istio-proxy", 0, false),
        ],
    });
    let worker = json!({
        "metadata": {
            "name": "worker-0", "namespace": "app-commandes", "creationTimestamp": ago(60 * 26),
            "ownerReferences": [{ "kind": "StatefulSet", "name": "worker", "controller": true }],
        },
        "spec": { "containers": [{ "name": "worker" }] },
        "status": { "phase": "Running", "containerStatuses": [running("worker", 2, true)] },
    });
    vec![
        ("app-commandes", vec![deployment_pod("api-commandes", "7d9f8b6c5d", "x2k4p"), crashing, worker]),
        ("default", vec![]),
        ("kube-system", vec![]),
    ]
}

/// Horodatage RFC3339Nano comme Kubernetes (zéros finaux supprimés).
fn nano_ts(ms: i64, extra: usize) -> String {
    let iso = crate::text::iso(chrono::DateTime::from_timestamp_millis(ms).unwrap());
    let nanos = format!("{}{:06}", &iso[20..23], extra % 1_000_000);
    let nanos = nanos.trim_end_matches('0');
    if nanos.is_empty() { format!("{}Z", &iso[..19]) } else { format!("{}.{nanos}Z", &iso[..19]) }
}

const LEVELS: [&str; 7] = ["INFO", "INFO", "INFO", "DEBUG", "WARN", "INFO", "ERROR"];

/// Ligne réaliste selon le conteneur : JSON Logback, JSON pino, accès Envoy en texte.
fn line(container: &str, i: usize, ms: i64) -> String {
    let date = crate::text::iso(chrono::DateTime::from_timestamp_millis(ms).unwrap());
    match container {
        "istio-proxy" => {
            let status = if i % 17 == 0 { 503 } else { 200 };
            format!(
                "[{date}] \"GET /api/commandes/{} HTTP/1.1\" {status} - via_upstream - \"-\" 0 {} {} {} \"10.42.0.{}\" \"okhttp/4.12.0\" \"{:x}-trace\" \"api-commandes:8080\"",
                1000 + i % 50, 120 + i % 900, i % 40, i % 30, i % 250, i * 7919
            )
        }
        "migrations" => format!("Flyway: migrating schema \"commandes\" to version {} - ajout_colonne_{i}", 1 + i % 42),
        "worker" => {
            let level = [30, 30, 30, 20, 40, 30, 50][i % 7];
            json!({
                "level": level, "time": ms, "pid": 1, "hostname": "worker-0",
                "msg": if level >= 50 { "Échec du traitement du lot".to_string() } else { format!("Lot {i} traité") },
                "batch": { "id": i, "size": i % 100, "errors": if level >= 50 { json!(["timeout base", "retry épuisé"]) } else { json!([]) } },
            })
            .to_string()
        }
        _ => {
            let level = LEVELS[i % LEVELS.len()];
            let mut entry = json!({
                "@timestamp": date,
                "level": level,
                "logger_name": if i % 3 != 0 { "fr.acme.commandes.api.CommandeController" } else { "fr.acme.commandes.service.PaiementService" },
                "thread_name": format!("http-nio-8080-exec-{}", 1 + i % 10),
                "message": if level == "ERROR" { format!("Erreur lors du paiement de la commande {}", 1000 + i) } else {
                    format!("Commande {} {} en {} ms", 1000 + i, if i % 2 != 0 { "créée" } else { "mise à jour" }, 10 + i % 90)
                },
                "traceId": format!("{:016x}", i * 104729),
            });
            if level == "ERROR" {
                entry["stack_trace"] = json!(
                    "java.lang.IllegalStateException: Paiement refusé par la banque\n\tat fr.acme.commandes.service.PaiementService.payer(PaiementService.java:87)\n\tat fr.acme.commandes.api.CommandeController.valider(CommandeController.java:142)\n\tat java.base/java.lang.Thread.run(Thread.java:1583)\nCaused by: java.net.SocketTimeoutException: Read timed out\n\tat java.base/java.net.SocketInputStream.read(SocketInputStream.java:183)\n\t... 42 common frames omitted"
                );
            }
            if i % 5 == 0 {
                entry["payload"] = json!(json!({ "commande": 1000 + i, "client": { "id": i % 300, "nom": "Dupont" }, "lignes": [{ "sku": format!("SKU-{i}"), "qte": 1 + i % 3 }] }).to_string());
            }
            if i % 11 == 0 {
                entry["sql"] = json!("SELECT c.id, c.statut\nFROM commande c\nWHERE c.client_id = ?\nORDER BY c.date DESC");
            }
            entry.to_string()
        }
    }
}

fn ts_key(ts: &str) -> String { crate::streams::lines::ts_key(ts) }

struct State {
    control: Arc<Control>,
    logs: Mutex<Logs>,
}

impl State {
    fn live_pods(&self, ns: &str) -> Option<Vec<Value>> {
        let deleted = self.control.deleted.lock();
        fake_pods().into_iter().find(|(n, _)| *n == ns).map(|(_, pods)| {
            pods.into_iter().filter(|p| !deleted.contains(p["metadata"]["name"].as_str().unwrap_or_default())).collect()
        })
    }

    fn store(&self, key: &str, container: &str) -> Vec<Entry> {
        let mut logs = self.logs.lock();
        logs.stores
            .entry(key.into())
            .or_insert_with(|| {
                let now = crate::text::now_ms();
                let history = self.control.history.load(Ordering::Relaxed);
                (0..history)
                    .map(|i| {
                        let ms = now - (history - i) as i64 * 10;
                        Entry { ts: nano_ts(ms, i * 37), text: line(container, i, ms) }
                    })
                    .collect()
            })
            .clone()
    }

    fn start_ticker(self: &Arc<Self>) {
        {
            let mut logs = self.logs.lock();
            if logs.ticker { return; }
            logs.ticker = true;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(state) = weak.upgrade() else { break };
                let interval = state.control.interval_ms.load(Ordering::Relaxed);
                drop(state);
                tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
                let Some(state) = weak.upgrade() else { break };
                let burst = state.control.burst.load(Ordering::Relaxed);
                let history = state.control.history.load(Ordering::Relaxed);
                let mut logs = state.logs.lock();
                let keys: Vec<String> = logs.stores.keys().cloned().collect();
                for key in keys {
                    let container = key.split('/').nth(2).unwrap_or_default().to_string();
                    for _ in 0..burst {
                        let ms = crate::text::now_ms();
                        let seq = logs.seq;
                        logs.seq += 1;
                        let entry = Entry { ts: nano_ts(ms, seq % 1_000_000), text: line(&container, history + seq, ms) };
                        logs.stores.get_mut(&key).unwrap().push(entry.clone());
                        if let Some(ls) = logs.listeners.get_mut(&key) {
                            ls.retain(|l| l.send(entry.clone()).is_ok());
                        }
                    }
                    let max = 20_000usize.max(history);
                    let store = logs.stores.get_mut(&key).unwrap();
                    if store.len() > max { store.drain(..store.len() - max); }
                }
            }
        });
    }

    fn serve_logs(self: &Arc<Self>, req: &Req, pod: &str, has_previous: bool) -> Resp {
        let container = req.q("container").unwrap_or_default().to_string();
        let previous = req.q("previous") == Some("true");
        if previous && !has_previous {
            return Resp::Json(400, json!({
                "kind": "Status", "code": 400,
                "message": format!("previous terminated container \"{container}\" in pod \"{pod}\" not found"),
            }));
        }
        let key = format!("{pod}/{}/{container}", if previous { "previous" } else { "current" });
        let mut entries = self.store(&key, &container);
        if let Some(since_time) = req.q("sinceTime") {
            static FRAC: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"\.\d+Z$").unwrap());
            let from = ts_key(&FRAC.replace(since_time, "Z"));
            entries.retain(|e| ts_key(&e.ts) >= from);
        } else if let Some(s) = req.q("sinceSeconds").and_then(|s| s.parse::<i64>().ok()).filter(|s| *s > 0) {
            let from = ts_key(&crate::text::iso(chrono::Utc::now() - chrono::Duration::seconds(s)));
            entries.retain(|e| ts_key(&e.ts) >= from);
        }
        if let Some(tail) = req.q("tailLines") {
            let n: usize = tail.parse().unwrap_or(0);
            if n > 0 && n < entries.len() { entries.drain(..entries.len() - n); }
        }
        let timestamps = req.q("timestamps") == Some("true");
        let format = move |e: &Entry| if timestamps { format!("{} {}\n", e.ts, e.text) } else { format!("{}\n", e.text) };
        let (tx, rx) = mpsc::unbounded_channel();
        let initial: String = entries.iter().map(&format).collect();
        let _ = tx.send(Ok(Bytes::from(initial)));
        if req.q("follow") != Some("true") || previous || container == "migrations" {
            return Resp::Stream(200, "text/plain; charset=utf-8".into(), rx);
        }
        self.start_ticker();
        let (ltx, mut lrx) = mpsc::unbounded_channel::<Entry>();
        self.logs.lock().listeners.entry(key).or_default().push(ltx);
        let drop_after = self.control.drop_after_ms.load(Ordering::Relaxed);
        tokio::spawn(async move {
            let deadline = (drop_after > 0).then(|| tokio::time::Instant::now() + std::time::Duration::from_millis(drop_after));
            loop {
                tokio::select! {
                    _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                        let _ = tx.send(Err(std::io::Error::other("coupure simulée")));
                        break;
                    }
                    e = lrx.recv() => match e {
                        Some(e) => if tx.send(Ok(Bytes::from(format(&e)))).is_err() { break },
                        None => break,
                    },
                    _ = tx.closed() => break,
                }
            }
        });
        Resp::Stream(200, "text/plain; charset=utf-8".into(), rx)
    }

    fn k8s(self: &Arc<Self>, req: &Req, cluster: &str, path: &str) -> Option<Resp> {
        if cluster == "local" {
            return Some(Resp::Json(403, json!({ "kind": "Status", "code": 403, "message": "forbidden" })));
        }
        let not_found = || Resp::Json(404, json!({ "kind": "Status", "code": 404, "message": "not found" }));
        if path == "/api/v1/namespaces" {
            let items: Vec<Value> = fake_pods().iter().map(|(n, _)| json!({ "metadata": { "name": n }, "status": { "phase": "Active" } })).collect();
            return Some(Resp::Json(200, json!({ "items": items })));
        }
        let parts: Vec<&str> = path.trim_start_matches("/api/v1/namespaces/").split('/').collect();
        if !path.starts_with("/api/v1/namespaces/") { return None; }
        match parts.as_slice() {
            [ns, "pods"] => Some(match self.live_pods(ns) {
                Some(pods) => Resp::Json(200, json!({ "items": pods })),
                None => not_found(),
            }),
            [ns, "pods", name] => {
                Some(self.live_pods(ns).and_then(|p| p.into_iter().find(|p| p["metadata"]["name"] == *name)).map(|p| Resp::Json(200, p)).unwrap_or_else(not_found))
            }
            [ns, "pods", name, "log"] => {
                let Some(pod) = self.live_pods(ns).and_then(|p| p.into_iter().find(|p| p["metadata"]["name"] == *name)) else { return Some(not_found()) };
                let container = req.q("container").unwrap_or_default();
                let has_previous = pod["status"]["containerStatuses"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|c| c["name"] == container)
                    .map(|c| c["lastState"]["terminated"].is_object())
                    .unwrap_or(false);
                Some(self.serve_logs(req, name, has_previous))
            }
            _ => None,
        }
    }
}

/// Démarre le faux Rancher (port libre si 0).
pub async fn start(port: u16) -> FakeRancher {
    let (cert, key, der) = http::self_signed("fake-rancher");
    let control = Arc::new(Control::default());
    let state = Arc::new(State { control: control.clone(), logs: Mutex::new(Logs::default()) });
    let server = http::start(
        port,
        Some((cert.clone(), key)),
        http::handler(move |req: Req| {
            let state = state.clone();
            async move {
                if req.header("authorization") != Some(&format!("Bearer {FAKE_TOKEN}")) {
                    return Resp::Json(401, json!({ "type": "error", "status": 401, "message": "must authenticate" }));
                }
                if req.path == "/v3/users" && req.q("me") == Some("true") {
                    return Resp::Json(200, json!({ "data": [{ "id": "u-1", "username": "jdupont", "name": "Jean Dupont" }] }));
                }
                if req.path == "/v3/clusters" {
                    return Resp::Json(200, json!({ "data": fake_clusters() }));
                }
                if let Some(rest) = req.path.strip_prefix("/k8s/clusters/") {
                    if let Some((cluster, path)) = rest.split_once('/') {
                        if let Some(r) = state.k8s(&req, cluster, &format!("/{path}")) { return r; }
                    }
                }
                Resp::Json(404, json!({ "type": "error", "status": 404, "message": "not found" }))
            }
        }),
    )
    .await;
    FakeRancher { url: server.url.clone(), ca_pem: cert, ca_der: der, control, _server: server }
}
