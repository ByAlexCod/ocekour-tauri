//! Explorateur S3 : lectures simples et opérations longues (progression, annulation).

use super::client::{S3Client, to_source_error};
use super::{OpContext, Progress, S3Entry, S3Page, S3Preview};
use crate::analytics::{AnalyticsEvent, track};
use crate::config::{Connection, S3Connection};
use crate::connections::Connections;
use crate::error::SourceError;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Avancement envoyé à l'interface (`s3:progress`).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct S3Progress {
    pub op_id: String,
    #[serde(flatten)]
    pub progress: Progress,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub finished: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SourceError>,
}

/// Opération longue.
#[derive(Debug, Clone)]
pub enum S3Op {
    Download { entries: Vec<S3Entry>, file: Option<PathBuf>, dir: Option<PathBuf> },
    Upload { prefix: String, paths: Vec<PathBuf> },
    Rename { entry: S3Entry, new_name: String },
    Remove { entries: Vec<S3Entry> },
}

impl S3Op {
    fn kind(&self) -> &'static str {
        match self {
            S3Op::Download { .. } => "Téléchargement",
            S3Op::Upload { .. } => "Envoi",
            S3Op::Rename { .. } => "Renommage",
            S3Op::Remove { .. } => "Suppression",
        }
    }
}

pub type ProgressSink = Arc<dyn Fn(S3Progress) + Send + Sync>;

pub struct S3Service {
    connections: Arc<Connections>,
    clients: Mutex<HashMap<String, (Arc<S3Client>, S3Connection)>>,
    running: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

fn check_prefix(prefix: &str) -> Result<(), SourceError> {
    if prefix.is_empty() || prefix.ends_with('/') { Ok(()) } else { Err(SourceError::invalid("Préfixe de dossier attendu")) }
}

impl S3Service {
    pub fn new(connections: Arc<Connections>) -> Self {
        S3Service { connections, clients: Mutex::new(HashMap::new()), running: Arc::new(Mutex::new(HashMap::new())) }
    }

    fn resolve(&self, connection_id: &str) -> Result<(Arc<S3Client>, S3Connection), SourceError> {
        let connection = match self.connections.get(connection_id)? {
            Connection::S3(s) => s,
            other => return Err(SourceError::invalid(format!("{} n'est pas une connexion S3.", other.name()))),
        };
        let mut clients = self.clients.lock();
        if let Some((client, cached)) = clients.get(connection_id) {
            if *cached == connection { return Ok((client.clone(), connection)); }
        }
        let client = Arc::new(S3Client::new(&connection, &self.connections.secret(connection_id)?)?);
        clients.insert(connection_id.into(), (client.clone(), connection.clone()));
        Ok((client, connection))
    }

    pub async fn list(&self, connection_id: &str, bucket: &str, prefix: &str, search: &str, continuation: Option<&str>) -> Result<S3Page, SourceError> {
        check_prefix(prefix)?;
        let (client, _) = self.resolve(connection_id)?;
        super::list_folder(&client, bucket, prefix, search, continuation).await.map_err(|e| to_source_error(&e, bucket))
    }

    pub async fn preview(&self, connection_id: &str, bucket: &str, key: &str) -> Result<S3Preview, SourceError> {
        let (client, _) = self.resolve(connection_id)?;
        super::preview(&client, bucket, key).await.map_err(|e| to_source_error(&e, key))
    }

    pub async fn count(&self, connection_id: &str, bucket: &str, prefix: &str) -> Result<usize, SourceError> {
        check_prefix(prefix)?;
        let (client, _) = self.resolve(connection_id)?;
        super::count_keys(&client, bucket, prefix).await.map_err(|e| to_source_error(&e, bucket))
    }

    /// Lance une opération longue ; la progression part vers `sink` (~10/s au plus), puis
    /// un dernier avancement `finished`.
    pub fn start(&self, connection_id: &str, bucket: &str, label: &str, op: S3Op, sink: ProgressSink) -> Result<String, SourceError> {
        if let S3Op::Upload { prefix, .. } = &op { check_prefix(prefix)?; }
        let (client, connection) = self.resolve(connection_id)?;
        let what = format!("S3 {} › {bucket} : {label}", connection.name);
        log::info!("{what} — début");
        let op_id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        self.running.lock().insert(op_id.clone(), cancel.clone());
        let latest = Arc::new(Mutex::new((Progress::default(), None::<Instant>)));
        let report: Arc<dyn Fn(Progress) + Send + Sync> = {
            let (latest, sink, op_id) = (latest.clone(), sink.clone(), op_id.clone());
            Arc::new(move |p: Progress| {
                let mut l = latest.lock();
                l.0 = p.clone();
                if l.1.is_none_or(|t| t.elapsed() > Duration::from_millis(100)) {
                    l.1 = Some(Instant::now());
                    drop(l);
                    sink(S3Progress { op_id: op_id.clone(), progress: p, ..Default::default() });
                }
            })
        };
        let ctx = OpContext { client, bucket: bucket.into(), cancel: cancel.clone(), report };
        let kind = op.kind();
        let id = op_id.clone();
        let finished_ops = self.running.clone();
        let bucket_name = bucket.to_string();
        tokio::spawn(async move {
            let result = match &op {
                S3Op::Download { entries, file, dir } => super::download(&ctx, entries, file.as_deref(), dir.as_deref()).await,
                S3Op::Upload { prefix, paths } => super::upload(&ctx, prefix, paths).await,
                S3Op::Rename { entry, new_name } => super::rename(&ctx, entry, new_name).await,
                S3Op::Remove { entries } => super::remove(&ctx, entries).await,
            };
            let progress = latest.lock().0.clone();
            let mut last = S3Progress { op_id: id.clone(), progress: progress.clone(), finished: true, ..Default::default() };
            match result {
                Ok(()) if !ctx.cancel.is_cancelled() => {
                    log::info!("{what} — terminé ({} objet(s))", progress.done);
                    track(AnalyticsEvent::new("S3", kind, Some("terminé")).value(progress.done as f64));
                }
                result => {
                    let cancelled = ctx.cancel.is_cancelled();
                    if cancelled {
                        log::info!("{what} — annulé");
                    } else if let Err(e) = &result {
                        log::error!("{what} — échec : {e}");
                        last.error = Some(to_source_error(e, &bucket_name));
                    }
                    track(AnalyticsEvent::new("S3", kind, Some(if cancelled { "annulé" } else { "échec" })));
                    last.cancelled = cancelled;
                }
            }
            finished_ops.lock().remove(&id);
            sink(last);
        });
        Ok(op_id)
    }

    pub fn cancel(&self, op_id: &str) {
        if let Some(c) = self.running.lock().get(op_id) { c.cancel(); }
    }

    /// Connexion modifiée ou supprimée : les clients seront recréés.
    pub fn reset(&self) { self.clients.lock().clear(); }
}
