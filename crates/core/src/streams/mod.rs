//! Flux de lignes (conteneur Rancher, topic Kafka) : événements envoyés par lots au
//! destinataire (fenêtre, outil MCP, téléchargement).

pub mod lines;
pub mod rancher;
pub mod kafka;

use crate::error::SourceError;
use crate::logs::LogLine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamState {
    Connecting,
    Live,
    Reconnecting,
    Ended,
    Error,
}

impl StreamState {
    pub fn as_str(self) -> &'static str {
        match self {
            StreamState::Connecting => "connecting",
            StreamState::Live => "live",
            StreamState::Reconnecting => "reconnecting",
            StreamState::Ended => "ended",
            StreamState::Error => "error",
        }
    }
    pub fn is_final(self) -> bool { matches!(self, StreamState::Ended | StreamState::Error) }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StreamEvent {
    Lines { lines: Vec<LogLine> },
    Status {
        state: StreamState,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<SourceError>,
        /// Pod supprimé : pods actuels du même workload.
        #[serde(skip_serializing_if = "Option::is_none")]
        replacements: Option<Vec<String>>,
    },
    /// Événement à insérer dans le flux (redémarrage…).
    Notice { message: String },
}

impl StreamEvent {
    pub fn status(state: StreamState, message: Option<String>) -> Self {
        StreamEvent::Status { state, message, error: None, replacements: None }
    }
}

/// Destinataire des événements d'un flux.
pub type Emit = Arc<dyn Fn(StreamEvent) + Send + Sync>;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamOptions {
    /// Suivi en temps réel ; sinon historique seul.
    pub follow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_lines: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<bool>,
}

/// Temporisations (réduites dans les tests).
#[derive(Debug, Clone)]
pub struct Timing {
    pub batch: Duration,
    pub batch_lines: usize,
    pub retry_delays: Vec<Duration>,
    /// Coupe un flux silencieux (connexion morte sans erreur TCP).
    pub idle_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            batch: Duration::from_millis(100),
            batch_lines: 1_000,
            retry_delays: [1_000, 2_000, 5_000, 10_000].iter().map(|ms| Duration::from_millis(*ms)).collect(),
            idle_timeout: Duration::from_secs(5 * 60),
        }
    }
}

impl Timing {
    pub fn retry_delay(&self, attempt: usize) -> Duration {
        self.retry_delays[attempt.min(self.retry_delays.len() - 1)]
    }
}
