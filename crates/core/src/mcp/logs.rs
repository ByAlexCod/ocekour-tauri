//! Lecture des logs pour le serveur MCP et les téléchargements : même flux, même analyse et
//! mêmes filtres que l'interface.

use crate::app::App;
use crate::config::Connection;
use crate::logs::parser::Level;
use crate::logs::{LogEntry, LogLine, Matcher, SearchOptions, filter_entries, resolve_profile};
use crate::streams::{StreamEvent, StreamOptions, StreamState};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct ReadRequest {
    pub connection_id: String,
    /// Rancher : cluster, namespace, pod, conteneur ; Kafka : topic.
    pub path: Vec<String>,
    pub options: StreamOptions,
    /// Suivi en direct pendant ce nombre de secondes après l'historique (0 : historique seul).
    pub follow_seconds: u64,
    pub search: Option<SearchOptions>,
    pub levels: Option<Vec<Level>>,
    pub max_lines: usize,
}

pub struct ReadResult {
    pub entries: Vec<LogEntry>,
    /// Lignes reçues avant filtrage.
    pub received: usize,
    pub status: StreamState,
    pub message: Option<String>,
    pub profile: String,
}

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Délai maximal d'un historique (sans suivi).
pub const HISTORY_LIMIT: Duration = Duration::from_secs(120);

/// Ouvre un flux, collecte ses lignes jusqu'à la fin de l'historique (ou du suivi).
pub async fn read_logs(app: &App, request: ReadRequest) -> anyhow::Result<ReadResult> {
    let connection = app.connections.get(&request.connection_id)?;
    let lines: Arc<Mutex<Vec<LogLine>>> = Arc::default();
    let last: Arc<Mutex<(StreamState, Option<String>)>> = Arc::new(Mutex::new((StreamState::Connecting, None)));
    let done = Arc::new(tokio::sync::Notify::new());
    let following = request.follow_seconds > 0;
    let owner = format!("lecture-{}", NEXT_OWNER.fetch_add(1, Ordering::Relaxed));
    let send = {
        let (lines, last, done) = (lines.clone(), last.clone(), done.clone());
        Arc::new(move |_: &str, event: StreamEvent| match event {
            StreamEvent::Lines { lines: l } => lines.lock().extend(l),
            StreamEvent::Status { state, message, error, .. } => {
                let message = message.or_else(|| error.map(|e| e.message));
                *last.lock() = (state, message);
                if state.is_final() { done.notify_one(); }
            }
            StreamEvent::Notice { .. } => {}
        })
    };
    let options = StreamOptions { follow: following, ..request.options.clone() };
    let id = app.streams.open(&owner, &request.connection_id, &request.path, options, send)?;
    let limit = if following { Duration::from_secs(request.follow_seconds) } else { HISTORY_LIMIT };
    let _ = tokio::time::timeout(limit, done.notified()).await;
    app.streams.close(&id);

    let key = profile_key_of(app, &connection, &request.path).await;
    let profile = resolve_profile(&app.config.config().profiles, &key);
    let lines = std::mem::take(&mut *lines.lock());
    let received = lines.len();
    let entries: Vec<LogEntry> = lines.into_iter().map(|l| LogEntry::new(l, &profile.fields)).collect();
    let matcher = match &request.search {
        Some(s) if !s.text.is_empty() => Matcher::build(s)?,
        _ => None,
    };
    let levels = request.levels.as_deref().filter(|l| !l.is_empty());
    let mut matching = filter_entries(entries, levels, matcher.as_ref());
    if matching.len() > request.max_lines { matching.drain(..matching.len() - request.max_lines); }
    let (status, message) = last.lock().clone();
    Ok(ReadResult { entries: matching, received, status, message, profile: key })
}

/// Clé de profil (mêmes réglages que l'interface) : `cluster/namespace/workload` ou `kafka/…`.
pub async fn profile_key_of(app: &App, connection: &Connection, path: &[String]) -> String {
    match connection {
        Connection::Kafka(k) => format!("kafka/{}/{}", k.name, path.first().cloned().unwrap_or_default()),
        Connection::Rancher(r) => {
            let (cluster_id, namespace, pod) = (path[0].clone(), path.get(1).cloned().unwrap_or_default(), path.get(2).cloned().unwrap_or_default());
            let cluster = r.clusters.iter().find(|c| c.id == cluster_id).map(|c| c.name.clone()).unwrap_or(cluster_id.clone());
            let workload = workload_of(app, &r.id, &cluster_id, &namespace, &pod).await;
            format!("{cluster}/{namespace}/{}", workload.unwrap_or(pod))
        }
        Connection::S3(_) => String::new(),
    }
}

/// Workload d'un pod (pods du namespace parcourus).
pub async fn workload_of(app: &App, connection_id: &str, cluster_id: &str, namespace: &str, pod: &str) -> Option<String> {
    let pods = crate::sources::browse(&app.connections, connection_id, &[cluster_id.into(), namespace.into()]).await.ok()?;
    pods.into_iter().find(|p| p.id == pod).and_then(|p| p.workload).filter(|w| !w.is_empty())
}
