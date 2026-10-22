//! Flux ouverts, rattachés à leur destinataire (fenêtre, outil MCP, téléchargement).

use super::{Emit, StreamEvent, StreamOptions, StreamState, Timing};
use crate::analytics::{AnalyticsEvent, track};
use crate::config::Connection;
use crate::connections::Connections;
use crate::error::SourceError;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct Entry {
    cancel: CancellationToken,
    owner: String,
    label: String,
}

pub struct StreamManager {
    connections: Arc<Connections>,
    streams: Mutex<HashMap<String, Entry>>,
    timing: Timing,
}

impl StreamManager {
    pub fn new(connections: Arc<Connections>) -> Self { Self::with_timing(connections, Timing::default()) }

    pub fn with_timing(connections: Arc<Connections>, timing: Timing) -> Self {
        StreamManager { connections, streams: Mutex::new(HashMap::new()), timing }
    }

    /// Ouvre un flux ; ses événements partent vers `send(id, event)`.
    /// Rancher : [cluster, namespace, pod, conteneur, workload?] ; Kafka : [topic].
    pub fn open(&self, owner: &str, connection_id: &str, path: &[String], options: StreamOptions, send: Arc<dyn Fn(&str, StreamEvent) + Send + Sync>) -> Result<String, SourceError> {
        if path.is_empty() || path.len() > 5 || path.iter().any(|p| p.is_empty()) {
            return Err(SourceError::invalid("Chemin de flux invalide."));
        }
        if options.since_seconds == Some(0) { return Err(SourceError::invalid("sinceSeconds doit être ≥ 1.")); }
        let local = connection_id == crate::files::LOCAL;
        let connection = if local { None } else { Some(self.connections.get(connection_id)?) };
        let id = uuid::Uuid::new_v4().to_string();
        let kind = connection.as_ref().map(|c| c.kind()).unwrap_or("fichier");
        let label = match &connection {
            Some(c) => format!("{} › {}", c.name(), path.iter().take(4).cloned().collect::<Vec<_>>().join("/")),
            None => format!("Fichier › {}", path.join(" › ")),
        };
        let emit: Emit = {
            let (id, label) = (id.clone(), label.clone());
            Arc::new(move |event: StreamEvent| {
                if let StreamEvent::Status { state, message, error, .. } = &event {
                    if *state == StreamState::Error {
                        track(AnalyticsEvent::new("Erreur", &format!("flux {kind}"), Some(error.as_ref().map(|e| e.code.as_str()).unwrap_or("inconnue"))));
                    }
                    if matches!(state, StreamState::Error | StreamState::Ended | StreamState::Reconnecting) {
                        let line = format!("Flux {label} : {}{}", state.as_str(), message.as_ref().map(|m| format!(" — {m}")).unwrap_or_default());
                        if *state == StreamState::Ended { log::info!("{line}") } else { log::warn!("{line}") }
                    }
                }
                send(&id, event);
            })
        };
        let cancel = CancellationToken::new();
        let encoding = match options.encoding.as_deref() {
            None | Some("") | Some("auto") => None,
            Some(e) => Some(super::encoding::Encoding::parse(e).ok_or_else(|| SourceError::invalid(format!("Encodage inconnu : {e}")))?),
        };
        let Some(connection) = connection else {
            let target = crate::files::target_of(path)?;
            let file_options = super::file::FileOptions {
                follow: options.follow,
                tail_lines: options.tail_lines,
                since_seconds: options.since_seconds,
                encoding,
                resume: None,
            };
            spawn_file(target, file_options, emit, cancel.clone());
            return Ok(self.register(id, cancel, owner, label, kind, &options));
        };
        match &connection {
            Connection::Rancher(_) => {
                if path.len() < 4 { return Err(SourceError::invalid("Chemin Rancher incomplet : cluster, namespace, pod, conteneur.")); }
                let r = super::rancher::ContainerRef {
                    cluster_id: path[0].clone(), namespace: path[1].clone(), pod: path[2].clone(), container: path[3].clone(),
                    workload: path.get(4).cloned(),
                };
                super::rancher::spawn(self.connections.target(connection_id)?, r, options.clone(), emit, self.timing.clone(), cancel.clone());
            }
            Connection::Kafka(k) => {
                if path.len() != 1 { return Err(SourceError::invalid("Chemin Kafka : [topic].")); }
                super::kafka::spawn(k.clone(), self.connections.secret(connection_id)?, path[0].clone(), options.clone(), emit, self.timing.clone(), cancel.clone());
            }
            Connection::S3(_) => return Err(SourceError::invalid(format!("{} : pas de flux de logs.", connection.name()))),
            Connection::Sonde(s) => {
                crate::sonde::client::spawn(s.clone(), self.connections.secret(connection_id)?, path, options.clone(), emit, self.timing.clone(), cancel.clone())?;
            }
        }
        Ok(self.register(id, cancel, owner, label, kind, &options))
    }

    fn register(&self, id: String, cancel: CancellationToken, owner: &str, label: String, kind: &str, options: &StreamOptions) -> String {
        log::info!("Flux {label} ouvert {}", serde_json::to_string(options).unwrap_or_default());
        let mut event = AnalyticsEvent::new(
            "Flux",
            "Ouvert",
            Some(&format!("{kind} · {}{}", if options.follow { "suivi" } else { "historique" }, if options.previous == Some(true) { " · précédent" } else { "" })),
        );
        if let Some(t) = options.tail_lines { event = event.value(t as f64); }
        track(event);
        self.streams.lock().insert(id.clone(), Entry { cancel, owner: owner.into(), label });
        id
    }

    pub fn close(&self, id: &str) {
        if let Some(entry) = self.streams.lock().remove(id) {
            log::info!("Flux {} fermé", entry.label);
            entry.cancel.cancel();
        }
    }

    /// Fenêtre fermée ou rechargée : ses flux sont arrêtés.
    pub fn close_all_of(&self, owner: &str) {
        let ids: Vec<String> = self.streams.lock().iter().filter(|(_, e)| e.owner == owner).map(|(id, _)| id.clone()).collect();
        for id in ids { self.close(&id); }
    }

    pub fn count(&self) -> usize { self.streams.lock().len() }
}

/// Fichier (du poste) : la lecture tourne dans un fil dédié, ses événements partent vers `emit`.
pub fn spawn_file(target: super::file::FileTarget, options: super::file::FileOptions, emit: Emit, cancel: CancellationToken) {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    super::file::spawn(target, options, super::file::FileTiming::default(), cancel, tx);
    tokio::spawn(async move {
        while let Some(e) = rx.recv().await { emit(e.event); }
    });
}
