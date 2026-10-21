//! Contexte IA : Rocket.Chat (tout message), GitLab, Jira et Confluence sondés à intervalle
//! régulier ; les nouveautés traversent un pipeline « classer → curer (LLM facultatif) →
//! écrire » vers la mémoire de contexte. Indépendant des notifications.

pub mod agents;
pub mod entity;
pub mod llm;
pub mod rocketchat;
pub mod state;
pub mod store;
pub mod tools;

use crate::analytics::AnalyticsEvent;
use crate::config::{CONTEXT_SOURCES, ConfigStore, ContexteConfig};
use crate::notifications::state::{GitlabCursor, SinceCursor};
use crate::notifications::{Clients, Track, confluence, gitlab, jira};
use entity::{ContextEvent, event_from_item};
use llm::Curator;
use parking_lot::Mutex;
use serde::Serialize;
use state::ContextState;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use store::ContextStore;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextStatus {
    pub enabled: bool,
    pub running: bool,
    pub interval_seconds: u64,
    pub sources: Vec<String>,
    pub entities: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Sources « joignables » (URL + jeton) : indépendamment des réglages MCP.
pub trait Reachable: Send + Sync {
    fn reachable(&self, id: &str) -> bool;
}

impl Reachable for crate::services::Accounts {
    fn reachable(&self, id: &str) -> bool { self.settings(id).is_some_and(|s| !s.url.is_empty()) && self.has_token(id) }
}

/// Découverte du LLM (remplaçable dans les tests).
pub type Discover = Arc<dyn Fn(&str) -> Option<llm::LlmConfig> + Send + Sync>;

#[derive(Default)]
struct Last {
    run: Option<String>,
    count: Option<usize>,
    error: Option<String>,
    llm: Option<String>,
}

pub struct ContextEngine {
    clients: Arc<dyn Clients>,
    reachable: Arc<dyn Reachable>,
    config: Arc<ConfigStore>,
    state: tokio::sync::Mutex<ContextState>,
    store: Arc<ContextStore>,
    track: Track,
    usable: Arc<dyn Fn() -> bool + Send + Sync>,
    discover: Discover,
    last: Mutex<Last>,
    ticking: AtomicBool,
    timer: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Pipeline d'ingestion : classer (ordre chronologique), curer (LLM, jamais bloquant), écrire.
pub async fn ingest(store: &ContextStore, curator: &Curator, mut events: Vec<ContextEvent>) -> usize {
    events.sort_by(|a, b| a.at.cmp(&b.at));
    if curator.available() {
        let curated = futures_util::future::join_all(events.iter().map(|e| async {
            match e.body.as_deref().filter(|b| !b.trim().is_empty()) {
                Some(body) => curator.curate(body, &format!("{} — {}", e.entity.title, e.summary)).await,
                None => None,
            }
        }))
        .await;
        for (e, resume) in events.iter_mut().zip(curated) { if let Some(r) = resume { e.body = Some(r); } }
    }
    let mut written = 0;
    for e in &events {
        match store.ingest(e) {
            Ok(()) => written += 1,
            Err(err) => log::warn!("Contexte : écriture d’un événement : {err}"),
        }
    }
    written
}

impl ContextEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clients: Arc<dyn Clients>,
        reachable: Arc<dyn Reachable>,
        config: Arc<ConfigStore>,
        state: ContextState,
        store: Arc<ContextStore>,
        track: Track,
        usable: Arc<dyn Fn() -> bool + Send + Sync>,
        discover: Discover,
    ) -> Arc<Self> {
        Arc::new(ContextEngine {
            clients, reachable, config, state: tokio::sync::Mutex::new(state), store, track, usable, discover,
            last: Mutex::default(), ticking: AtomicBool::new(false), timer: Mutex::new(None),
        })
    }

    fn settings(&self) -> ContexteConfig { self.config.config().contexte.clone() }

    /// Sources activées ET joignables.
    fn sources(&self) -> Vec<String> {
        let settings = self.settings();
        CONTEXT_SOURCES.iter().filter(|s| settings.sources.get(**s).is_none_or(|t| t.enabled) && self.reachable.reachable(s)).map(|s| s.to_string()).collect()
    }

    pub fn status(&self) -> ContextStatus {
        let settings = self.settings();
        let last = self.last.lock();
        ContextStatus {
            enabled: settings.enabled,
            running: self.timer.lock().is_some(),
            interval_seconds: settings.interval_seconds,
            sources: self.sources(),
            entities: self.store.size(),
            llm: last.llm.clone(),
            last_run: last.run.clone(),
            last_count: last.count,
            last_error: last.error.clone(),
        }
    }

    pub fn apply(self: &Arc<Self>) {
        self.stop();
        let settings = self.settings();
        if !settings.enabled { return; }
        self.store.load();
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
        log::info!("Contexte IA : actif ({} s), sources : {}", settings.interval_seconds, { let s = self.sources(); if s.is_empty() { "aucune".into() } else { s.join(", ") } });
    }

    pub fn stop(&self) {
        if let Some(t) = self.timer.lock().take() { t.abort(); }
    }

    pub async fn tick(&self) {
        if !self.settings().enabled || !(self.usable)() { return; }
        if self.ticking.swap(true, Ordering::SeqCst) { return; }
        let result = self.run().await;
        let mut last = self.last.lock();
        match result {
            Ok(()) => last.error = None,
            Err(e) => {
                log::warn!("Contexte IA : {e}");
                last.error = Some(e.to_string());
            }
        }
        drop(last);
        self.ticking.store(false, Ordering::SeqCst);
    }

    async fn run(&self) -> anyhow::Result<()> {
        let now = crate::text::now_iso();
        let sources = self.sources();
        let mut state = self.state.lock().await;
        if !state.initialized() {
            self.baseline(&mut state, &now, &sources).await;
            return Ok(());
        }
        let mut events = vec![];
        for source in &sources {
            match self.fetch_source(&mut state, source, &now).await {
                Ok(e) => events.extend(e),
                Err(e) => log::warn!("Contexte IA {source} : {e}"),
            }
        }
        let fresh: Vec<ContextEvent> = events.into_iter().filter(|e| state.is_new(&e.dedup_key)).collect();
        self.last.lock().run = Some(now.clone());
        if fresh.is_empty() {
            self.last.lock().count = Some(0);
        } else {
            let settings = self.settings();
            let curator = Curator::new((self.discover)(&settings.llm), settings.tls.clone());
            self.last.lock().llm = curator.describe();
            let keys: Vec<String> = fresh.iter().map(|e| e.dedup_key.clone()).collect();
            let written = ingest(&self.store, &curator, fresh).await;
            state.mark_seen(keys.iter().map(String::as_str));
            self.last.lock().count = Some(written);
            (self.track)(AnalyticsEvent::new("Contexte", "ingestion", None).value(written as f64));
        }
        state.save();
        Ok(())
    }

    /// Premier passage : pose les curseurs au présent, sans rien ingérer.
    async fn baseline(&self, state: &mut ContextState, now: &str, sources: &[String]) {
        for source in sources {
            let result: anyhow::Result<()> = async {
                match source.as_str() {
                    "gitlab" => {
                        let last_id = gitlab::baseline(&self.clients.client("gitlab")?).await?;
                        state.cursors_mut().gitlab = Some(GitlabCursor { last_id: Some(last_id), since: Some(now.into()) });
                    }
                    "jira" => state.cursors_mut().jira = Some(SinceCursor { since: now.into() }),
                    "confluence" => state.cursors_mut().confluence = Some(SinceCursor { since: now.into() }),
                    _ => {
                        let (_, rooms) = rocketchat::fetch(&self.clients.client("rocketchat")?, state.rooms(), now, true).await?;
                        state.set_rooms(rooms);
                    }
                }
                Ok(())
            }
            .await;
            if let Err(e) = result { log::warn!("Contexte IA {source} (départ) : {e}"); }
        }
        state.mark_initialized();
        self.last.lock().run = Some(now.into());
        state.save();
        log::info!("Contexte IA : démarré (part de maintenant, sans rattrapage).");
    }

    async fn fetch_source(&self, state: &mut ContextState, source: &str, now: &str) -> anyhow::Result<Vec<ContextEvent>> {
        let http = self.clients.client(source)?;
        let items = match source {
            "rocketchat" => {
                let (events, rooms) = rocketchat::fetch(&http, state.rooms(), now, false).await?;
                state.set_rooms(rooms);
                return Ok(events);
            }
            "gitlab" => {
                let events: Vec<String> = gitlab::default_events().into_iter().map(Into::into).collect();
                let cursor = state.cursors().gitlab.clone().unwrap_or_default();
                let ((todos, last_id), (notes, since)) = tokio::try_join!(
                    gitlab::fetch_todos(&http, cursor.last_id.unwrap_or(0), &events),
                    gitlab::fetch_notes(&http, cursor.since.as_deref(), &events, now),
                )?;
                state.cursors_mut().gitlab = Some(GitlabCursor { last_id: Some(last_id), since: Some(since) });
                todos.into_iter().chain(notes).collect::<Vec<_>>()
            }
            "jira" => {
                let events: Vec<String> = jira::DEFAULT_EVENTS.iter().map(|s| s.to_string()).collect();
                let since = state.cursors().jira.as_ref().map(|c| c.since.clone());
                let (items, next) = jira::fetch(&http, since.as_deref(), &events, now).await?;
                state.cursors_mut().jira = Some(SinceCursor { since: next });
                items
            }
            _ => {
                let events: Vec<String> = confluence::DEFAULT_EVENTS.iter().map(|s| s.to_string()).collect();
                let since = state.cursors().confluence.as_ref().map(|c| c.since.clone());
                let (items, next) = confluence::fetch(&http, since.as_deref(), &events, now).await?;
                state.cursors_mut().confluence = Some(SinceCursor { since: next });
                items
            }
        };
        Ok(items.iter().filter_map(|i| event_from_item(i, now)).collect())
    }
}
