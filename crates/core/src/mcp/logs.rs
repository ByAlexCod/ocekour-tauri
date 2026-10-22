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
    /// Historique : attendre la fin tant que des lignes arrivent (téléchargement complet), au
    /// lieu de s'arrêter après `HISTORY_LIMIT` (réponse rapide aux agents).
    pub until_end: bool,
}

pub struct ReadResult {
    pub entries: Vec<LogEntry>,
    /// Lignes reçues avant filtrage.
    pub received: usize,
    pub status: StreamState,
    pub message: Option<String>,
    pub profile: String,
    /// Plus de lignes que `max_lines` : les plus anciennes ont été écartées.
    pub truncated: bool,
    /// Rancher : le pod n'existe plus ; pods actuels de son workload.
    pub replacements: Option<Vec<String>>,
}

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Délai maximal d'un historique (sans suivi) ; avec `until_end`, délai sans nouvelle ligne.
pub const HISTORY_LIMIT: Duration = Duration::from_secs(120);

/// Ouvre un flux, collecte ses lignes jusqu'à la fin de l'historique (ou du suivi).
pub async fn read_logs(app: &App, request: ReadRequest) -> anyhow::Result<ReadResult> {
    let local = request.connection_id == crate::files::LOCAL;
    let connection = if local { None } else { Some(app.connections.get(&request.connection_id)?) };
    let lines: Arc<Mutex<Vec<LogLine>>> = Arc::default();
    let last: Arc<Mutex<(StreamState, Option<String>)>> = Arc::new(Mutex::new((StreamState::Connecting, None)));
    let replaced: Arc<Mutex<Option<Vec<String>>>> = Arc::default();
    let done = Arc::new(tokio::sync::Notify::new());
    let following = request.follow_seconds > 0;
    let owner = format!("lecture-{}", NEXT_OWNER.fetch_add(1, Ordering::Relaxed));
    let send = {
        let (lines, last, done, replaced) = (lines.clone(), last.clone(), done.clone(), replaced.clone());
        Arc::new(move |_: &str, event: StreamEvent| match event {
            StreamEvent::Lines { lines: l } => lines.lock().extend(l),
            StreamEvent::Status { state, message, error, replacements } => {
                if replacements.is_some() { *replaced.lock() = replacements; }
                let message = message.or_else(|| error.map(|e| e.message));
                *last.lock() = (state, message);
                if state.is_final() { done.notify_one(); }
            }
            StreamEvent::Notice { .. } => {}
        })
    };
    let mut options = StreamOptions { follow: following, ..request.options.clone() };
    // Fichier du poste : encodage mémorisé pour ce fichier (choisi dans son onglet).
    if local && options.encoding.is_none() {
        options.encoding = request.path.first().and_then(|p| app.files.encoding_of(p)).map(|e| e.as_str().to_string());
    }
    let id = app.streams.open(&owner, &request.connection_id, &request.path, options, send)?;
    if following {
        let _ = tokio::time::timeout(Duration::from_secs(request.follow_seconds), done.notified()).await;
    } else {
        loop {
            let before = lines.lock().len();
            if tokio::time::timeout(HISTORY_LIMIT, done.notified()).await.is_ok() { break; }
            // Historique long (gros topic, gros fichier) : on attend tant qu'il progresse.
            if !request.until_end || lines.lock().len() == before { break; }
        }
    }
    app.streams.close(&id);

    let key = match &connection {
        Some(c) => profile_key_of(app, c, &request.path).await,
        None => crate::files::profile_key(&request.path),
    };
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
    let truncated = matching.len() > request.max_lines;
    if truncated { matching.drain(..matching.len() - request.max_lines); }
    let (status, message) = last.lock().clone();
    let replacements = replaced.lock().take();
    Ok(ReadResult { entries: matching, received, status, message, profile: key, truncated, replacements })
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
        Connection::Sonde(s) => format!("sonde/{}/{}", s.name, path.first().cloned().unwrap_or_default()),
    }
}

/// Workload d'un pod (pods du namespace parcourus).
pub async fn workload_of(app: &App, connection_id: &str, cluster_id: &str, namespace: &str, pod: &str) -> Option<String> {
    let pods = crate::sources::browse(&app.connections, connection_id, &[cluster_id.into(), namespace.into()]).await.ok()?;
    pods.into_iter().find(|p| p.id == pod).and_then(|p| p.workload).filter(|w| !w.is_empty())
}
