//! Notifications : GitLab, Jira et Confluence interrogés à intervalle régulier ; ce qui
//! concerne directement l'utilisateur part en un message groupé à soi-même dans Rocket.Chat
//! (et en notification système). N'utilise jamais le chemin d'écriture des agents.

pub mod confluence;
pub mod gitlab;
pub mod jira;
pub mod message;
pub mod state;

use crate::analytics::AnalyticsEvent;
use crate::config::{ConfigStore, NOTIFICATION_SOURCES, NotificationsConfig};
use crate::services::http::{ServiceError, ServiceHttp};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;
use state::{GitlabCursor, NotifItem, NotificationState, SinceCursor};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Comptes des services (jetons de l'utilisateur) : `Accounts`, ou un double de test.
pub trait Clients: Send + Sync {
    fn available(&self, id: &str) -> bool;
    fn client(&self, id: &str) -> Result<ServiceHttp, ServiceError>;
}

impl Clients for crate::services::Accounts {
    fn available(&self, id: &str) -> bool { crate::services::Accounts::available(self, id) }
    fn client(&self, id: &str) -> Result<ServiceHttp, ServiceError> { crate::services::Accounts::client(self, id) }
}

/// Notification native (barre système).
pub type DesktopNotify = Arc<dyn Fn(&str, &str) + Send + Sync>;
pub type Track = Arc<dyn Fn(AnalyticsEvent) + Send + Sync>;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotifierStatus {
    pub enabled: bool,
    pub running: bool,
    pub interval_seconds: u64,
    pub target: String,
    /// Sources activées ET munies d'un jeton.
    pub actives: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Default)]
struct Last {
    run: Option<String>,
    count: Option<usize>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TestOutcome {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Notifier {
    clients: Arc<dyn Clients>,
    config: Arc<ConfigStore>,
    state: tokio::sync::Mutex<NotificationState>,
    track: Track,
    usable: Arc<dyn Fn() -> bool + Send + Sync>,
    desktop: Option<DesktopNotify>,
    last: Mutex<Last>,
    warned: Mutex<HashSet<String>>,
    ticking: AtomicBool,
    timer: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

fn label(source: &str) -> &'static str {
    match source { "gitlab" => "GitLab", "jira" => "Jira", _ => "Confluence" }
}

/// Retire les doublons de clé (garde le premier : la file en attente d'abord).
fn dedup(items: Vec<NotifItem>) -> Vec<NotifItem> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|i| seen.insert(i.dedup_key.clone())).collect()
}

impl Notifier {
    pub fn new(
        clients: Arc<dyn Clients>,
        config: Arc<ConfigStore>,
        state: NotificationState,
        track: Track,
        usable: Arc<dyn Fn() -> bool + Send + Sync>,
        desktop: Option<DesktopNotify>,
    ) -> Arc<Self> {
        Arc::new(Notifier {
            clients, config, state: tokio::sync::Mutex::new(state), track, usable, desktop,
            last: Mutex::default(), warned: Mutex::default(), ticking: AtomicBool::new(false), timer: Mutex::new(None),
        })
    }

    fn settings(&self) -> NotificationsConfig { self.config.config().notifications.clone() }

    fn desktop_on(&self) -> bool { self.settings().desktop && self.desktop.is_some() }

    /// Sources activées dans la configuration ET disposant d'un jeton.
    fn actives(&self) -> Vec<String> {
        let settings = self.settings();
        NOTIFICATION_SOURCES.iter().filter(|s| settings.sources.get(**s).is_some_and(|t| t.enabled) && self.clients.available(s)).map(|s| s.to_string()).collect()
    }

    pub fn status(&self) -> NotifierStatus {
        let settings = self.settings();
        let last = self.last.lock();
        NotifierStatus {
            enabled: settings.enabled,
            running: self.timer.lock().is_some(),
            interval_seconds: settings.interval_seconds,
            target: settings.target,
            actives: self.actives(),
            last_run: last.run.clone(),
            last_count: last.count,
            last_error: last.error.clone(),
        }
    }

    /// (Re)programme la boucle selon la configuration.
    pub fn apply(self: &Arc<Self>) {
        self.stop();
        let settings = self.settings();
        if !settings.enabled { return; }
        let weak = Arc::downgrade(self);
        let every = Duration::from_secs(settings.interval_seconds);
        *self.timer.lock() = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            loop {
                let Some(me) = weak.upgrade() else { break };
                me.tick().await;
                drop(me);
                tokio::time::sleep(every).await;
            }
        }));
        log::info!("Notifications : actives ({} s), sources : {}", settings.interval_seconds, { let a = self.actives(); if a.is_empty() { "aucune".into() } else { a.join(", ") } });
    }

    pub fn stop(&self) {
        if let Some(t) = self.timer.lock().take() { t.abort(); }
    }

    /// Un passage : lit les sources, envoie le message groupé, met à jour l'état.
    pub async fn tick(&self) {
        if !self.settings().enabled || !(self.usable)() { return; }
        if self.ticking.swap(true, Ordering::SeqCst) { return; }
        if let Err(e) = self.run().await {
            log::warn!("Notifications : {e}");
            self.last.lock().error = Some(e.to_string());
        }
        self.ticking.store(false, Ordering::SeqCst);
    }

    async fn run(&self) -> anyhow::Result<()> {
        let can_send = self.clients.available("rocketchat");
        if !can_send && !self.desktop_on() {
            if self.warned.lock().insert("aucun-canal".into()) {
                log::warn!("Notifications : ni Rocket.Chat ni notifications système, rien à envoyer.");
            }
            return Ok(());
        }
        let now = crate::text::now_iso();
        let mut state = self.state.lock().await;
        if !state.initialized() { return self.baseline(&mut state, &now, can_send).await; }

        let mut fresh: Vec<NotifItem> = vec![];
        for source in self.actives() {
            match self.fetch_source(&mut state, &source, &now).await {
                Ok(items) => {
                    fresh.extend(items.into_iter().filter(|i| state.is_new(&i.dedup_key)));
                    self.warned.lock().remove(&source);
                }
                Err(e) => {
                    log::warn!("Notifications {source} : {e}");
                    if self.warned.lock().insert(source.clone()) {
                        // Le message d'erreur nomme déjà le service (« GitLab : … »).
                        fresh.push(NotifItem { source: "ocekour".into(), kind: "erreur".into(), title: e.to_string(), dedup_key: format!("ocekour:{source}-error"), ..Default::default() });
                    }
                }
            }
        }
        self.last.lock().run = Some(now.clone());
        // Notifications natives : les nouveautés de ce passage, indépendamment de Rocket.Chat.
        if self.desktop_on() && !fresh.is_empty() {
            let (title, body) = message::plain_summary(&fresh);
            (self.desktop.as_ref().unwrap())(&title, &body);
        }
        let to_send = dedup(state.pending().iter().cloned().chain(fresh.iter().cloned()).collect());
        if to_send.is_empty() {
            let mut last = self.last.lock();
            last.count = Some(0);
            last.error = None;
            drop(last);
            state.save();
            return Ok(());
        }
        if can_send {
            match self.send(&message::build_message(&to_send).unwrap()).await {
                Ok(()) => {
                    state.mark_seen(to_send.iter().map(|i| i.dedup_key.as_str()));
                    state.clear_pending();
                    let mut last = self.last.lock();
                    last.count = Some(to_send.len());
                    last.error = None;
                    drop(last);
                    self.track_counts(&to_send);
                }
                Err(e) => {
                    // Rocket.Chat injoignable : regroupé au passage suivant.
                    state.queue(&fresh);
                    self.last.lock().error = Some(e.to_string());
                    log::warn!("Notifications : envoi Rocket.Chat impossible : {e}");
                }
            }
        } else {
            // Sans Rocket.Chat : le bureau a délivré les nouveautés.
            state.mark_seen(fresh.iter().map(|i| i.dedup_key.as_str()));
            let mut last = self.last.lock();
            last.count = Some(fresh.len());
            last.error = None;
            drop(last);
            self.track_counts(&fresh);
        }
        state.save();
        Ok(())
    }

    fn track_counts(&self, items: &[NotifItem]) {
        for source in NOTIFICATION_SOURCES {
            let n = items.iter().filter(|i| i.source == source).count();
            if n > 0 { (self.track)(AnalyticsEvent::new("Notifications", source, None).value(n as f64)); }
        }
    }

    /// Premier passage : curseurs au présent, message de confirmation, rien d'autre.
    async fn baseline(&self, state: &mut NotificationState, now: &str, can_send: bool) -> anyhow::Result<()> {
        let sources = self.actives();
        let mut gitlab_id = None;
        if sources.iter().any(|s| s == "gitlab") { gitlab_id = Some(gitlab::baseline(&self.clients.client("gitlab")?).await?); }
        let names = if sources.is_empty() { "aucune source configurée".to_string() } else { sources.iter().map(|s| label(s)).collect::<Vec<_>>().join(", ") };
        if can_send {
            self.send(&format!("🔔 **Ocekour — notifications activées**\nVous recevrez ici les nouveautés qui vous concernent : {names}.")).await?;
        }
        if self.desktop_on() { (self.desktop.as_ref().unwrap())("Ocekour — notifications activées", &format!("Sources surveillées : {names}.")); }
        for source in &sources {
            let c = state.cursors_mut();
            match source.as_str() {
                "gitlab" => c.gitlab = Some(GitlabCursor { last_id: gitlab_id, since: Some(now.into()) }),
                "jira" => c.jira = Some(SinceCursor { since: now.into() }),
                _ => c.confluence = Some(SinceCursor { since: now.into() }),
            }
        }
        state.mark_initialized();
        self.last.lock().run = Some(now.into());
        state.save();
        log::info!("Notifications : activées, message de confirmation envoyé.");
        Ok(())
    }

    fn events_for(&self, source: &str) -> Vec<String> {
        let chosen = self.settings().sources.get(source).map(|s| s.events.clone()).unwrap_or_default();
        if !chosen.is_empty() { return chosen; }
        match source {
            "gitlab" => gitlab::default_events().into_iter().map(Into::into).collect(),
            "jira" => jira::DEFAULT_EVENTS.iter().map(|s| s.to_string()).collect(),
            _ => confluence::DEFAULT_EVENTS.iter().map(|s| s.to_string()).collect(),
        }
    }

    async fn fetch_source(&self, state: &mut NotificationState, source: &str, now: &str) -> anyhow::Result<Vec<NotifItem>> {
        let http = self.clients.client(source)?;
        let events = self.events_for(source);
        match source {
            "gitlab" => {
                let cursor = state.cursors().gitlab.clone().unwrap_or_default();
                // Boîte To-Do (assigné, mention, pipeline…) et commentaires de MR.
                let ((todos, last_id), (notes, since)) = tokio::try_join!(
                    gitlab::fetch_todos(&http, cursor.last_id.unwrap_or(0), &events),
                    gitlab::fetch_notes(&http, cursor.since.as_deref(), &events, now),
                )?;
                state.cursors_mut().gitlab = Some(GitlabCursor { last_id: Some(last_id), since: Some(since) });
                Ok(todos.into_iter().chain(notes).collect())
            }
            "jira" => {
                let since = state.cursors().jira.as_ref().map(|c| c.since.clone());
                let (items, next) = jira::fetch(&http, since.as_deref(), &events, now).await?;
                state.cursors_mut().jira = Some(SinceCursor { since: next });
                Ok(items)
            }
            _ => {
                let since = state.cursors().confluence.as_ref().map(|c| c.since.clone());
                let (items, next) = confluence::fetch(&http, since.as_deref(), &events, now).await?;
                state.cursors_mut().confluence = Some(SinceCursor { since: next });
                Ok(items)
            }
        }
    }

    /// Envoi Rocket.Chat (auto-message par défaut).
    pub async fn send(&self, text: &str) -> anyhow::Result<()> {
        let rc = self.clients.client("rocketchat")?;
        let mut channel = self.settings().target;
        if channel == "self" { channel = format!("@{}", crate::services::s(&rc.get("api/v1/me", vec![]).await?["username"])); }
        rc.post("api/v1/chat.postMessage", json!({ "channel": channel, "text": text })).await?;
        Ok(())
    }

    /// Envoi d'essai (bouton « Tester »).
    pub async fn test(&self) -> TestOutcome {
        if !self.clients.available("rocketchat") {
            return TestOutcome { ok: false, error: Some("Rocket.Chat n’est pas configuré (Paramètres → Comptes).".into()) };
        }
        match self.send("🔔 **Ocekour — message d’essai**\nLes notifications GitLab, Jira et Confluence arriveront ici.").await {
            Ok(()) => {
                if self.desktop_on() { (self.desktop.as_ref().unwrap())("Ocekour — message d’essai", "Les notifications système fonctionnent."); }
                TestOutcome { ok: true, error: None }
            }
            Err(e) => TestOutcome { ok: false, error: Some(e.to_string()) },
        }
    }
}
