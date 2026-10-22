//! Statistiques d'utilisation vers un Matomo auto-hébergé (API HTTP de suivi, en lots).
//!
//! Ce qui part : écrans vus, actions, codes d'erreur, version. Ce qui ne part JAMAIS : secrets,
//! URL ou noms de serveurs, clusters, pods, topics, buckets, textes recherchés, contenu des
//! logs. Le visiteur est un identifiant aléatoire local.

use crate::config::Tls;
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AnalyticsEvent {
    pub category: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

impl AnalyticsEvent {
    pub fn new(category: &str, action: &str, name: Option<&str>) -> Self {
        AnalyticsEvent { category: category.into(), action: action.into(), name: name.map(str::to_string), value: None }
    }
    pub fn value(mut self, v: f64) -> Self { self.value = Some(v); self }
}

/// Destination effective (fournie par la clé d'activation) ; `None` = pas de suivi.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub url: String,
    pub site_id: u64,
    pub insecure: bool,
    pub dim_version: Option<u64>,
    pub dim_key: Option<u64>,
    pub key_id: Option<String>,
}

pub struct Options {
    pub target: Box<dyn Fn() -> Option<Target> + Send + Sync>,
    pub visitor_file: PathBuf,
    pub version: String,
    pub user_agent: String,
    pub language: String,
    pub resolution: Option<String>,
    pub flush: Duration,
    pub heartbeat: Duration,
    /// Après chaque tentative d'envoi : joint (2xx) ou non.
    pub on_delivery: Option<Box<dyn Fn(bool) + Send + Sync>>,
}

struct Queued {
    query: String,
    at: i64,
}

struct State {
    queue: Vec<Queued>,
    new_visit: bool,
    current_url: String,
    visitor: Option<String>,
}

const MAX_QUEUE: usize = 500;
const MAX_AGE_MS: i64 = 60 * 60_000;

fn clip(text: &str, max: usize) -> String { text.chars().take(max).collect() }

pub struct Analytics {
    options: Options,
    state: Mutex<State>,
    /// Dernier envoi accepté (ms), 0 si aucun.
    last_delivery: AtomicI64,
    sending: tokio::sync::Mutex<()>,
    stop: tokio_util::sync::CancellationToken,
}

impl Analytics {
    pub fn new(options: Options) -> Arc<Self> {
        Arc::new(Analytics {
            options,
            state: Mutex::new(State { queue: vec![], new_visit: true, current_url: "app://ocekour/".into(), visitor: None }),
            last_delivery: AtomicI64::new(0),
            sending: tokio::sync::Mutex::new(()),
            stop: tokio_util::sync::CancellationToken::new(),
        })
    }

    pub fn last_delivery(&self) -> i64 { self.last_delivery.load(Ordering::Relaxed) }

    pub fn enabled(&self) -> bool { (self.options.target)().is_some() }

    /// Envois réguliers et signal de présence (durée d'utilisation).
    pub fn start(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        let (flush, heartbeat) = (self.options.flush, self.options.heartbeat);
        let stop = self.stop.clone();
        tokio::spawn(async move {
            let mut f = tokio::time::interval(flush);
            let mut h = tokio::time::interval(heartbeat);
            f.tick().await;
            h.tick().await;
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = f.tick() => { if let Some(a) = me.upgrade() { a.flush().await } else { break } }
                    _ = h.tick() => { if let Some(a) = me.upgrade() { a.enqueue(vec![("ping", "1".into())]) } else { break } }
                }
            }
        });
    }

    /// Écran affiché : `path` sans donnée identifiante.
    pub fn page(&self, title: &str, path: &str) {
        self.state.lock().current_url = format!("app://ocekour/{}", clip(path.trim_start_matches('/'), 150));
        self.enqueue(vec![("action_name", clip(title, 150))]);
    }

    pub fn event(&self, e: &AnalyticsEvent) {
        let mut params = vec![("e_c", clip(&e.category, 150)), ("e_a", clip(&e.action, 150))];
        if let Some(n) = e.name.as_deref().filter(|n| !n.is_empty()) { params.push(("e_n", clip(n, 150))); }
        if let Some(v) = e.value.filter(|v| v.is_finite()) {
            params.push(("e_v", if v.fract() == 0.0 { format!("{}", v as i64) } else { v.to_string() }));
        }
        self.enqueue(params);
    }

    /// Vérifie que Matomo répond (clé « suivi obligatoire »).
    pub async fn check(&self) -> bool {
        if !self.enabled() { return false; }
        if self.state.lock().queue.is_empty() { self.enqueue(vec![("ping", "1".into())]); }
        let before = self.last_delivery();
        self.flush().await;
        self.last_delivery() > before
    }

    fn visitor_id(&self) -> String {
        let file = &self.options.visitor_file;
        if let Ok(id) = std::fs::read_to_string(file) {
            let id = id.trim();
            if id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) { return id.into(); }
        }
        let mut raw = [0u8; 8];
        rand::rng().fill_bytes(&mut raw);
        let id = hex::encode(raw);
        if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
        let _ = std::fs::write(file, &id);
        id
    }

    fn enqueue(&self, params: Vec<(&str, String)>) {
        let Some(target) = (self.options.target)() else {
            self.state.lock().queue.clear();
            return;
        };
        let mut state = self.state.lock();
        if state.visitor.is_none() { state.visitor = Some(self.visitor_id()); }
        let now = chrono::Local::now();
        let mut rand = [0u8; 4];
        rand::rng().fill_bytes(&mut rand);
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        q.append_pair("idsite", &target.site_id.to_string())
            .append_pair("rec", "1")
            .append_pair("apiv", "1")
            .append_pair("send_image", "0")
            .append_pair("_id", state.visitor.as_deref().unwrap())
            .append_pair("rand", &hex::encode(rand))
            .append_pair("url", &state.current_url)
            .append_pair("ua", &self.options.user_agent)
            .append_pair("lang", &self.options.language)
            .append_pair("h", &now.format("%-H").to_string())
            .append_pair("m", &now.format("%-M").to_string())
            .append_pair("s", &now.format("%-S").to_string());
        if let Some(r) = &self.options.resolution { q.append_pair("res", r); }
        if let Some(d) = target.dim_version { q.append_pair(&format!("dimension{d}"), &self.options.version); }
        if let (Some(d), Some(k)) = (target.dim_key, &target.key_id) { q.append_pair(&format!("dimension{d}"), k); }
        if state.new_visit { q.append_pair("new_visit", "1"); }
        for (k, v) in &params { q.append_pair(k, v); }
        state.new_visit = false;
        state.queue.push(Queued { query: format!("?{}", q.finish()), at: now.timestamp_millis() });
        let len = state.queue.len();
        if len > MAX_QUEUE { state.queue.drain(..len - MAX_QUEUE); }
    }

    /// Envoie la file ; en cas d'échec, elle est gardée pour la prochaine fois.
    pub async fn flush(&self) {
        let _guard = self.sending.lock().await;
        loop {
            let Some(target) = (self.options.target)() else {
                self.state.lock().queue.clear();
                return;
            };
            let batch: Vec<Queued> = {
                let mut state = self.state.lock();
                let limit = crate::text::now_ms() - MAX_AGE_MS;
                state.queue.retain(|q| q.at >= limit);
                if state.queue.is_empty() { return; }
                let n = state.queue.len().min(100);
                state.queue.drain(..n).collect()
            };
            let body = serde_json::json!({ "requests": batch.iter().map(|q| q.query.clone()).collect::<Vec<_>>() }).to_string();
            if post(&target, body).await.is_err() {
                let mut state = self.state.lock();
                let rest = std::mem::take(&mut state.queue);
                state.queue = batch.into_iter().chain(rest).collect();
                drop(state);
                if let Some(cb) = &self.options.on_delivery { cb(false); }
                return;
            }
            self.last_delivery.store(crate::text::now_ms(), Ordering::Relaxed);
            if let Some(cb) = &self.options.on_delivery { cb(true); }
        }
    }

    /// Arrêt de l'application : dernier envoi (2 s au plus).
    pub async fn stop(&self) {
        self.stop.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.flush()).await;
    }
}

/// POST du lot vers `<url>/matomo.php`.
async fn post(target: &Target, body: String) -> anyhow::Result<()> {
    let url = crate::rancher::join(&target.url, "matomo.php")?;
    let client = crate::net::http_client(&Tls { insecure: target.insecure, ca_file: None, fingerprint: None })?;
    let res = client.post(url).header("content-type", "application/json").body(body).timeout(Duration::from_secs(10)).send().await?;
    if res.status().as_u16() >= 300 { anyhow::bail!("Matomo : HTTP {}", res.status().as_u16()); }
    Ok(())
}

static INSTANCE: once_cell::sync::OnceCell<Arc<Analytics>> = once_cell::sync::OnceCell::new();

/// Instance de l'application (aucune dans les tests : `track` ne fait rien).
pub fn install(analytics: Arc<Analytics>) { let _ = INSTANCE.set(analytics); }

pub fn track(event: AnalyticsEvent) {
    if let Some(a) = INSTANCE.get() { a.event(&event); }
}

pub fn track_page(title: &str, path: &str) {
    if let Some(a) = INSTANCE.get() { a.page(title, path); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::http::{self, Req, Resp};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;

    struct Matomo {
        received: Arc<Mutex<Vec<HashMap<String, String>>>>,
        fail_next: Arc<AtomicBool>,
        url: String,
        _server: http::TestServer,
    }

    async fn matomo() -> Matomo {
        let received = Arc::new(Mutex::new(vec![]));
        let fail_next = Arc::new(AtomicBool::new(false));
        let (r, f) = (received.clone(), fail_next.clone());
        let server = http::start(0, None, http::handler(move |req: Req| {
            let (r, f) = (r.clone(), f.clone());
            async move {
                let ok = req.path == "/matomo/matomo.php" && !f.swap(false, Ordering::SeqCst);
                if ok {
                    for q in req.json()["requests"].as_array().unwrap() {
                        let q = q.as_str().unwrap().trim_start_matches('?');
                        r.lock().push(url::form_urlencoded::parse(q.as_bytes()).into_owned().collect());
                    }
                }
                Resp::Empty(if ok { 204 } else { 503 }, vec![])
            }
        }))
        .await;
        Matomo { received, fail_next, url: format!("{}/matomo", server.url), _server: server }
    }

    fn make(m: &Matomo, dir: &std::path::Path, enabled: Arc<AtomicBool>, dimension: Option<u64>) -> Arc<Analytics> {
        let url = m.url.clone();
        Analytics::new(Options {
            target: Box::new(move || enabled.load(Ordering::SeqCst).then(|| Target { url: url.clone(), site_id: 7, insecure: false, dim_version: dimension, dim_key: None, key_id: None })),
            visitor_file: dir.join("visiteur"),
            version: "0.1.1".into(),
            user_agent: "Ocekour/0.1.1".into(),
            language: "fr-FR".into(),
            resolution: Some("1920x1080".into()),
            flush: Duration::from_secs(30),
            heartbeat: Duration::from_secs(300),
            on_delivery: None,
        })
    }

    #[tokio::test]
    async fn lots_visites_file_et_troncature() {
        let m = matomo().await;
        let dir = tempfile::tempdir().unwrap();
        let on = Arc::new(AtomicBool::new(true));
        let stats = make(&m, dir.path(), on.clone(), Some(3));
        stats.page("Logs", "logs/kafka");
        stats.event(&AnalyticsEvent::new("Flux", "Ouvert", Some("kafka · suivi")).value(1000.0));
        stats.flush().await;
        {
            let r = m.received.lock();
            assert_eq!(r.len(), 2);
            let (page, event) = (&r[0], &r[1]);
            for (k, v) in [("idsite", "7"), ("rec", "1"), ("action_name", "Logs"), ("url", "app://ocekour/logs/kafka"), ("new_visit", "1"), ("lang", "fr-FR"), ("res", "1920x1080"), ("dimension3", "0.1.1")] {
                assert_eq!(page.get(k).map(String::as_str), Some(v), "{k}");
            }
            assert!(regex::Regex::new("^[0-9a-f]{16}$").unwrap().is_match(&page["_id"]));
            for (k, v) in [("e_c", "Flux"), ("e_a", "Ouvert"), ("e_n", "kafka · suivi"), ("e_v", "1000"), ("url", "app://ocekour/logs/kafka")] {
                assert_eq!(event.get(k).map(String::as_str), Some(v), "{k}");
            }
            assert_eq!(event["_id"], page["_id"]);
            assert!(!event.contains_key("new_visit"));
        }
        // Même visiteur au lancement suivant.
        m.received.lock().clear();
        let second = make(&m, dir.path(), on.clone(), None);
        second.event(&AnalyticsEvent::new("Application", "Démarrage", None));
        second.flush().await;
        let first_id = { m.received.lock()[0]["_id"].clone() };
        let stats_id = std::fs::read_to_string(dir.path().join("visiteur")).unwrap();
        assert_eq!(first_id, stats_id);
        // Matomo injoignable : file gardée.
        m.received.lock().clear();
        m.fail_next.store(true, Ordering::SeqCst);
        stats.event(&AnalyticsEvent::new("Export", "ndjson", None));
        stats.flush().await;
        assert!(m.received.lock().is_empty());
        stats.event(&AnalyticsEvent::new("Export", "text", None));
        stats.flush().await;
        assert_eq!(m.received.lock().iter().map(|q| q["e_a"].clone()).collect::<Vec<_>>(), vec!["ndjson", "text"]);
        // Troncature.
        m.received.lock().clear();
        stats.event(&AnalyticsEvent::new("Erreur", &"x".repeat(1000), None));
        stats.flush().await;
        assert_eq!(m.received.lock()[0]["e_a"].len(), 150);
        // Désactivé : rien de gardé.
        m.received.lock().clear();
        on.store(false, Ordering::SeqCst);
        stats.event(&AnalyticsEvent::new("Export", "ndjson", None));
        on.store(true, Ordering::SeqCst);
        stats.flush().await;
        assert!(m.received.lock().is_empty());
        assert!(stats.check().await);
    }
}
