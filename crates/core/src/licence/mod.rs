//! Clé d'activation : lecture, vérification, stockage, et état de l'application (non activée,
//! vérification, active, bloquée). Voir docs/specs/cle-activation.md.

pub mod format;
pub mod keys;
pub mod preset;

use crate::analytics::Target;
use crate::secrets::SecretStore;
use format::{Keyring, LicenceError};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// Entrée du coffre à secrets qui contient la clé.
pub const LICENCE_SECRET_ID: &str = "licence:activation";
/// « Suivi obligatoire » : blocage sans envoi réussi depuis ce délai.
pub const REQUIRED_GRACE_MS: i64 = 15 * 60_000;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LicenceInfo {
    pub id: String,
    pub label: String,
    pub issued_at: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LicenceState {
    /// `missing`, `checking`, `active` ou `blocked`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<LicenceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub development: Option<bool>,
    /// Réponse à l'activation : préconfiguration ajoutée à config.yaml.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<Value>,
}

impl LicenceState {
    fn of(status: &'static str) -> Self { LicenceState { status, info: None, message: None, development: None, preset: None } }
}

/// Ce que la licence attend des statistiques (Matomo) pour le suivi obligatoire.
#[async_trait::async_trait]
pub trait Reachability: Send + Sync {
    async fn check(&self) -> bool;
    fn last_delivery(&self) -> i64;
}

#[async_trait::async_trait]
impl Reachability for crate::analytics::Analytics {
    async fn check(&self) -> bool { crate::analytics::Analytics::check(self).await }
    fn last_delivery(&self) -> i64 { crate::analytics::Analytics::last_delivery(self) }
}

/// Clés acceptées par cette version.
#[derive(Clone)]
pub struct LicenceKeys {
    pub keyring: Keyring,
    pub revoked: Vec<String>,
    /// Build de développement : clés de dev acceptées.
    pub development: bool,
}

impl LicenceKeys {
    /// Clés de cette compilation (production, plus développement hors build de diffusion).
    pub fn of_build() -> Self {
        let mut public_keys: Vec<(String, String)> = keys::PRODUCTION_PUBLIC_KEYS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut obfuscation_keys: Vec<String> = keys::OBFUSCATION_KEY.into_iter().map(str::to_string).collect();
        if !keys::RELEASE {
            public_keys.extend(keys::DEV_PUBLIC_KEYS.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            obfuscation_keys.push(keys::DEV_OBFUSCATION_KEY.into());
        }
        LicenceKeys {
            keyring: Keyring { public_keys, obfuscation_keys },
            revoked: keys::REVOKED_KEY_IDS.iter().map(|s| s.to_string()).collect(),
            development: !keys::RELEASE,
        }
    }
}

type Listener = Box<dyn Fn(&LicenceState) + Send + Sync>;

pub struct LicenceManager {
    secrets: Arc<SecretStore>,
    keys: LicenceKeys,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
    payload: Mutex<Option<Value>>,
    current: Mutex<LicenceState>,
    reachability: Mutex<Option<Arc<dyn Reachability>>>,
    listeners: Mutex<Vec<Listener>>,
    watchdog: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl LicenceManager {
    pub fn new(secrets: Arc<SecretStore>, keys: LicenceKeys) -> Arc<Self> {
        Self::with_clock(secrets, keys, Box::new(crate::text::now_ms))
    }

    pub fn with_clock(secrets: Arc<SecretStore>, keys: LicenceKeys, now: Box<dyn Fn() -> i64 + Send + Sync>) -> Arc<Self> {
        Arc::new(LicenceManager {
            secrets,
            keys,
            now,
            payload: Mutex::new(None),
            current: Mutex::new(LicenceState::of("missing")),
            reachability: Mutex::new(None),
            listeners: Mutex::new(vec![]),
            watchdog: Mutex::new(None),
        })
    }

    pub fn development(&self) -> bool { self.keys.development }

    pub fn state(&self) -> LicenceState { self.current.lock().clone() }

    /// L'application est-elle utilisable ?
    pub fn usable(&self) -> bool { self.current.lock().status == "active" }

    pub fn on_change(&self, listener: impl Fn(&LicenceState) + Send + Sync + 'static) {
        self.listeners.lock().push(Box::new(listener));
    }

    /// Destination Matomo de la clé ; `None` sans clé ou pour « sans suivi ».
    pub fn analytics_target(&self) -> Option<Target> {
        let payload = self.payload.lock();
        let p = payload.as_ref()?;
        let m = p.get("matomo")?;
        Some(Target {
            url: m["url"].as_str()?.to_string(),
            site_id: m["siteId"].as_u64()?,
            insecure: m["insecure"].as_bool().unwrap_or(false),
            dim_version: m["dimensions"]["version"].as_u64(),
            dim_key: m["dimensions"]["key"].as_u64(),
            key_id: p["id"].as_str().map(str::to_string),
        })
    }

    /// Préconfiguration de la clé en cours (validée).
    pub fn preset(&self) -> preset::Preset {
        let payload = self.payload.lock();
        preset::validate(payload.as_ref().and_then(|p| p.get("preset"))).unwrap_or_default()
    }

    /// Au démarrage : clé enregistrée (si valide), puis vérification du suivi.
    pub async fn load(self: &Arc<Self>, reachability: Arc<dyn Reachability>) -> LicenceState {
        *self.reachability.lock() = Some(reachability);
        if self.secrets.has(LICENCE_SECRET_ID) && !self.secrets.readable(LICENCE_SECRET_ID) {
            return self.set(LicenceState {
                message: Some("La clé enregistrée est illisible sur ce poste : collez-la à nouveau.".into()),
                ..LicenceState::of("missing")
            });
        }
        let Some(stored) = self.secrets.get(LICENCE_SECRET_ID) else { return self.set(LicenceState::of("missing")) };
        match self.decode(&stored) {
            Ok(p) => *self.payload.lock() = Some(p),
            // Clé révoquée ou devenue illisible : nouvelle activation.
            Err(e) => return self.set(LicenceState { message: Some(e.0), ..LicenceState::of("missing") }),
        }
        self.verify().await
    }

    /// Clé collée dans l'écran d'activation.
    pub async fn activate(self: &Arc<Self>, text: &str) -> Result<LicenceState, LicenceError> {
        let payload = self.decode(text)?;
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        self.secrets.set(LICENCE_SECRET_ID, &compact).map_err(|e| LicenceError(e.to_string()))?;
        *self.payload.lock() = Some(payload);
        Ok(self.verify().await)
    }

    /// « Réessayer » sur l'écran de blocage.
    pub async fn retry(self: &Arc<Self>) -> LicenceState {
        if self.payload.lock().is_some() { self.verify().await } else { self.state() }
    }

    /// Résultat d'un envoi Matomo : lève le blocage du suivi obligatoire.
    pub fn on_delivery(&self, delivered: bool) {
        let required = self.payload.lock().as_ref().is_some_and(|p| p["mode"] == json!("required"));
        if required && delivered && self.state().status == "blocked" {
            let info = self.info();
            self.set(LicenceState { info, ..LicenceState::of("active") });
        }
    }

    pub fn stop(&self) {
        if let Some(w) = self.watchdog.lock().take() { w.abort(); }
    }

    fn decode(&self, text: &str) -> Result<Value, LicenceError> {
        let (payload, _) = format::decode(text, &self.keys.keyring)?;
        // Préconfiguration invalide pour cette version : refusée dès l'activation.
        preset::validate(payload.get("preset")).map_err(|e| LicenceError(e.to_string()))?;
        let id = payload["id"].as_str().unwrap_or_default();
        if self.keys.revoked.iter().any(|r| r == id) {
            return Err(LicenceError(format!("Cette clé ({id}) a été révoquée : demandez-en une nouvelle.")));
        }
        Ok(payload)
    }

    fn info(&self) -> Option<LicenceInfo> {
        let payload = self.payload.lock();
        let p = payload.as_ref()?;
        Some(LicenceInfo {
            id: p["id"].as_str()?.into(),
            label: p["label"].as_str()?.into(),
            issued_at: p["issuedAt"].as_str()?.into(),
            mode: p["mode"].as_str()?.into(),
        })
    }

    /// « Suivi obligatoire » : Matomo doit répondre ; sinon, blocage.
    async fn verify(self: &Arc<Self>) -> LicenceState {
        self.stop();
        let info = self.info();
        if info.as_ref().map(|i| i.mode.as_str()) != Some("required") {
            return self.set(LicenceState { info, ..LicenceState::of("active") });
        }
        self.set(LicenceState { info: info.clone(), ..LicenceState::of("checking") });
        let reachability = self.reachability.lock().clone();
        let reached = match &reachability { Some(r) => r.check().await, None => false };
        let me = Arc::downgrade(self);
        *self.watchdog.lock() = Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(m) = me.upgrade() else { break };
                m.watch();
            }
        }));
        if reached { self.set(LicenceState { info, ..LicenceState::of("active") }) } else { self.set(self.blocked()) }
    }

    fn watch(&self) {
        if self.state().status != "active" { return; }
        let Some(r) = self.reachability.lock().clone() else { return };
        if (self.now)() - r.last_delivery() > REQUIRED_GRACE_MS { self.set(self.blocked()); }
    }

    fn blocked(&self) -> LicenceState {
        LicenceState {
            info: self.info(),
            message: Some(
                "Votre clé impose l'envoi des statistiques d'utilisation, mais le serveur de statistiques de l'équipe ne répond pas. \
                 Vérifiez le réseau (VPN ?) puis réessayez."
                    .into(),
            ),
            ..LicenceState::of("blocked")
        }
    }

    fn set(&self, state: LicenceState) -> LicenceState {
        *self.current.lock() = state.clone();
        for l in self.listeners.lock().iter() { l(&state); }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{Encryptor, SecretStore};
    use format::tests::{fixture, payload};
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

    struct Plain;
    impl Encryptor for Plain {
        fn available(&self) -> bool { true }
        fn encrypt(&self, p: &str) -> anyhow::Result<Vec<u8>> { Ok(p.as_bytes().to_vec()) }
        fn decrypt(&self, d: &[u8]) -> anyhow::Result<String> { Ok(String::from_utf8(d.to_vec())?) }
    }
    struct Broken;
    impl Encryptor for Broken {
        fn available(&self) -> bool { true }
        fn encrypt(&self, p: &str) -> anyhow::Result<Vec<u8>> { Ok(p.as_bytes().to_vec()) }
        fn decrypt(&self, _: &[u8]) -> anyhow::Result<String> { anyhow::bail!("Decryption is not available") }
    }

    struct Reach {
        ok: AtomicBool,
        calls: AtomicUsize,
        last: AtomicI64,
    }
    #[async_trait::async_trait]
    impl Reachability for Reach {
        async fn check(&self) -> bool { self.calls.fetch_add(1, Ordering::SeqCst); self.ok.load(Ordering::SeqCst) }
        fn last_delivery(&self) -> i64 { self.last.load(Ordering::SeqCst) }
    }
    fn reach(ok: bool) -> Arc<Reach> { Arc::new(Reach { ok: AtomicBool::new(ok), calls: AtomicUsize::new(0), last: AtomicI64::new(0) }) }

    fn store(dir: &tempfile::TempDir) -> Arc<SecretStore> {
        Arc::new(SecretStore::new(dir.path().join("secrets.json"), Box::new(Plain), None))
    }

    fn keys(f: &format::tests::Fixture, revoked: &[&str]) -> LicenceKeys {
        LicenceKeys { keyring: f.keyring.clone(), revoked: revoked.iter().map(|s| s.to_string()).collect(), development: false }
    }

    fn encode(f: &format::tests::Fixture, p: &Value) -> String { format::encode(p, "prod1", &f.private, &f.obfuscation).unwrap() }

    #[tokio::test]
    async fn activation_et_memorisation() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let secrets = store(&dir);
        let m = LicenceManager::new(secrets.clone(), keys(&f, &[]));
        assert_eq!(m.load(reach(true)).await.status, "missing");
        let p = payload(json!({}));
        let key = encode(&f, &p);
        let state = m.activate(&format!("  {}\n{}  ", &key[..50], &key[50..])).await.unwrap();
        assert_eq!(state.status, "active");
        let info = state.info.unwrap();
        assert_eq!((info.id.as_str(), info.mode.as_str()), (p["id"].as_str().unwrap(), "optional"));
        assert_eq!(secrets.get(LICENCE_SECRET_ID).unwrap(), key);
        let t = m.analytics_target().unwrap();
        assert_eq!((t.url.as_str(), t.site_id, t.key_id.as_deref()), ("https://matomo.intra/", 12, p["id"].as_str()));
        let again = LicenceManager::new(secrets, keys(&f, &[]));
        assert_eq!(again.load(reach(true)).await.status, "active");
    }

    #[tokio::test]
    async fn sans_suivi_et_suivi_si_possible() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let m = LicenceManager::new(store(&dir), keys(&f, &[]));
        m.load(reach(false)).await;
        assert_eq!(m.activate(&encode(&f, &payload(json!({ "mode": "none", "matomo": null })))).await.unwrap().status, "active");
        assert!(m.analytics_target().is_none());
        let r = reach(false);
        let m = LicenceManager::new(store(&dir), keys(&f, &[]));
        m.load(r.clone()).await;
        assert_eq!(m.activate(&encode(&f, &payload(json!({})))).await.unwrap().status, "active");
        assert_eq!(r.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cle_revoquee() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let p = payload(json!({}));
        let secrets = store(&dir);
        LicenceManager::new(secrets.clone(), keys(&f, &[])).activate(&encode(&f, &p)).await.unwrap();
        let m = LicenceManager::new(secrets, keys(&f, &[p["id"].as_str().unwrap()]));
        let state = m.load(reach(true)).await;
        assert_eq!(state.status, "missing");
        assert!(state.message.unwrap().contains("révoquée"));
        assert!(m.activate(&encode(&f, &p)).await.unwrap_err().0.contains("révoquée"));
    }

    #[tokio::test]
    async fn coffre_illisible() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        store(&dir).set(LICENCE_SECRET_ID, "x").unwrap();
        let broken = Arc::new(SecretStore::new(dir.path().join("secrets.json"), Box::new(Broken), None));
        let state = LicenceManager::new(broken, keys(&f, &[])).load(reach(true)).await;
        assert_eq!(state.status, "missing");
        assert!(state.message.unwrap().contains("illisible"));
    }

    #[tokio::test]
    async fn suivi_obligatoire_bloque_puis_reessayer() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let m = LicenceManager::new(store(&dir), keys(&f, &[]));
        let r = reach(false);
        m.load(r.clone()).await;
        let states = Arc::new(Mutex::new(vec![]));
        let s = states.clone();
        m.on_change(move |st| s.lock().push(st.status));
        assert_eq!(m.activate(&encode(&f, &payload(json!({ "mode": "required" })))).await.unwrap().status, "blocked");
        assert_eq!(*states.lock(), vec!["checking", "blocked"]);
        assert!(!m.usable());
        r.ok.store(true, Ordering::SeqCst);
        assert_eq!(m.retry().await.status, "active");
        m.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn quinze_minutes_sans_envoi_bloque() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(1_000_000));
        let c = clock.clone();
        let m = LicenceManager::with_clock(store(&dir), keys(&f, &[]), Box::new(move || c.load(Ordering::SeqCst)));
        let r = reach(true);
        r.last.store(1_000_000, Ordering::SeqCst);
        m.load(r.clone()).await;
        m.activate(&encode(&f, &payload(json!({ "mode": "required" })))).await.unwrap();
        assert_eq!(m.state().status, "active");
        clock.fetch_add(REQUIRED_GRACE_MS + 1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(61)).await;
        assert_eq!(m.state().status, "blocked");
        m.on_delivery(true);
        assert_eq!(m.state().status, "active");
        m.stop();
    }
}
