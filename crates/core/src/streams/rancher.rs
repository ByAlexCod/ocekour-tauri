//! Flux de logs d'un conteneur : lignes envoyées par lots, reconnexion automatique sans
//! doublon, détection d'un redémarrage du conteneur ou de la suppression du pod.

use super::lines::{Deduplicator, LineSplitter, parse_k8s_line};
use super::{Emit, StreamEvent, StreamOptions, StreamState, Timing};
use crate::error::{ErrorCode, SourceError};
use crate::logs::LogLine;
use crate::rancher::{self, RancherTarget, enc, items, workload_of};
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct ContainerRef {
    pub cluster_id: String,
    pub namespace: String,
    pub pod: String,
    pub container: String,
    /// Workload connu (lien de partage) : pods de remplacement si le pod n'existe plus.
    pub workload: Option<String>,
}

struct Run {
    target: RancherTarget,
    r: ContainerRef,
    options: StreamOptions,
    emit: Emit,
    timing: Timing,
    cancel: CancellationToken,
    splitter: LineSplitter,
    dedup: Deduplicator,
    batch: Vec<LogLine>,
    ended: bool,
    attempt: usize,
    workload: Option<String>,
    restarts: Option<u64>,
}

/// Lance le flux ; il s'arrête à la fin (historique) ou quand `cancel` est déclenché.
pub fn spawn(target: RancherTarget, r: ContainerRef, options: StreamOptions, emit: Emit, timing: Timing, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
    let workload = r.workload.clone();
    let run = Run {
        target, r, options, emit, timing, cancel,
        splitter: LineSplitter::default(),
        dedup: Deduplicator::default(),
        batch: vec![],
        ended: false,
        attempt: 0,
        workload,
        restarts: None,
    };
    tokio::spawn(run.main())
}

enum End {
    /// Fin normale de la réponse.
    Done,
    Failed(SourceError),
}

impl Run {
    fn follow(&self) -> bool { self.options.follow && self.options.previous != Some(true) }

    fn stopped(&self) -> bool { self.ended || self.cancel.is_cancelled() }

    async fn main(mut self) {
        // Pod introuvable : le flux se termine (avec les pods de remplacement).
        let alive = self.inspect_pod().await.unwrap_or(true);
        if !alive { return; }
        let mut reconnecting = false;
        loop {
            if self.stopped() { break; }
            let end = self.connect(reconnecting).await;
            if self.cancel.is_cancelled() { self.flush_batch(); break; }
            for l in self.splitter.flush() { self.add_line(&l); }
            self.flush_batch();
            let error = match end { End::Done => None, End::Failed(e) => Some(e) };
            if let Some(e) = &error {
                if matches!(e.code, ErrorCode::Unauthorized | ErrorCode::Forbidden | ErrorCode::Tls) {
                    self.finish(StreamState::Error, e.message.clone(), Some(e.clone()), None);
                    break;
                }
            }
            if !self.follow() {
                match error {
                    Some(e) => self.finish(StreamState::Error, e.message.clone(), Some(e), None),
                    None => {
                        let m = if self.options.previous == Some(true) { "Fin des logs précédents" } else { "Fin de l’historique" };
                        self.finish(StreamState::Ended, m.into(), None, None);
                    }
                }
                break;
            }
            // Suivi : reconnexion après un délai, en relisant d'abord le pod.
            let mut reason = error.map(|e| e.message);
            loop {
                let delay = self.timing.retry_delay(self.attempt);
                self.attempt += 1;
                self.status(StreamState::Reconnecting, Some(match &reason {
                    Some(r) => format!("{r} — nouvelle tentative…"),
                    None => "Flux interrompu — nouvelle tentative…".into(),
                }));
                tokio::select! {
                    _ = self.cancel.cancelled() => return,
                    _ = tokio::time::sleep(delay) => {}
                }
                match self.inspect_pod().await {
                    Ok(true) => break,
                    Ok(false) => return,
                    Err(e) => reason = Some(e.message),
                }
            }
            reconnecting = true;
        }
    }

    async fn connect(&mut self, reconnecting: bool) -> End {
        self.status(if reconnecting { StreamState::Reconnecting } else { StreamState::Connecting }, None);
        let mut query: Vec<(&str, String)> = vec![("container", self.r.container.clone()), ("timestamps", "true".into())];
        if self.follow() { query.push(("follow", "true".into())); }
        if self.options.previous == Some(true) { query.push(("previous", "true".into())); }
        match (&self.dedup.last, reconnecting) {
            (Some(last), true) => query.push(("sinceTime", last.clone())),
            _ => {
                if let Some(t) = self.options.tail_lines { query.push(("tailLines", t.to_string())); }
                if let Some(s) = self.options.since_seconds.filter(|s| *s > 0) { query.push(("sinceSeconds", s.to_string())); }
            }
        }
        let qs = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str()))).finish();
        let path = format!(
            "k8s/clusters/{}/api/v1/namespaces/{}/pods/{}/log?{qs}",
            enc(&self.r.cluster_id), enc(&self.r.namespace), enc(&self.r.pod)
        );
        let res = tokio::select! {
            _ = self.cancel.cancelled() => return End::Done,
            r = rancher::request(&self.target, &path, self.timing.idle_timeout) => r,
        };
        let res = match res {
            Ok(r) => r,
            Err(e) => return End::Failed(e),
        };
        self.status(StreamState::Live, if self.follow() { None } else { Some("Chargement de l’historique…".into()) });
        let mut body = res.bytes_stream();
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let flush_at = deadline;
            tokio::select! {
                _ = self.cancel.cancelled() => return End::Done,
                _ = async { tokio::time::sleep_until(flush_at.unwrap()).await }, if flush_at.is_some() => {
                    deadline = None;
                    self.flush_batch();
                }
                chunk = tokio::time::timeout(self.timing.idle_timeout, body.next()) => match chunk {
                    Err(_) => return End::Failed(SourceError::new(ErrorCode::Timeout, format!(
                        "Pas de réponse de {} après {} s.", crate::net::host_of(&self.target.url), self.timing.idle_timeout.as_secs()))),
                    Ok(None) => return End::Done,
                    Ok(Some(Err(e))) => return End::Failed(SourceError::new(ErrorCode::Network, format!("Flux interrompu ({})", short(&e)))),
                    Ok(Some(Ok(bytes))) => {
                        self.attempt = 0;
                        for l in self.splitter.push(&bytes) { self.add_line(&l); }
                        if self.batch.len() >= self.timing.batch_lines {
                            deadline = None;
                            self.flush_batch();
                        } else if !self.batch.is_empty() && deadline.is_none() {
                            deadline = Some(tokio::time::Instant::now() + self.timing.batch);
                        }
                    }
                }
            }
        }
    }

    fn add_line(&mut self, text: &str) {
        let line = parse_k8s_line(text);
        if self.dedup.accept(&line) { self.batch.push(line); }
    }

    fn flush_batch(&mut self) {
        if self.batch.is_empty() { return; }
        let lines = std::mem::take(&mut self.batch);
        (self.emit)(StreamEvent::Lines { lines });
    }

    fn status(&self, state: StreamState, message: Option<String>) {
        if self.stopped() { return; }
        (self.emit)(StreamEvent::status(state, message));
    }

    fn finish(&mut self, state: StreamState, message: String, error: Option<SourceError>, replacements: Option<Vec<String>>) {
        if self.ended { return; }
        self.flush_batch();
        self.ended = true;
        (self.emit)(StreamEvent::Status { state, message: Some(message), error, replacements });
    }

    /// Relit le pod : mémorise le workload, signale un redémarrage, termine le flux (avec
    /// des pods de remplacement) si le pod a été supprimé ; renvoie `false` alors.
    async fn inspect_pod(&mut self) -> Result<bool, SourceError> {
        let path = format!("/api/v1/namespaces/{}/pods/{}", enc(&self.r.namespace), enc(&self.r.pod));
        let pod = match rancher::kube_get(&self.target, &self.r.cluster_id, &path).await {
            Ok(p) => p,
            Err(e) if e.status == Some(404) => {
                let replacements = self.replacements().await.unwrap_or_default();
                let message = if self.restarts.is_none() {
                    format!("Le pod {} n'existe plus.", self.r.pod)
                } else {
                    format!("Le pod {} a été supprimé.", self.r.pod)
                };
                self.finish(StreamState::Ended, message, None, Some(replacements));
                return Ok(false);
            }
            Err(e) => return Err(e.error),
        };
        self.workload = Some(workload_of(&pod));
        let st = &pod["status"];
        let restarts = st["initContainerStatuses"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(st["containerStatuses"].as_array().into_iter().flatten())
            .find(|s| s["name"].as_str() == Some(&self.r.container))
            .and_then(|s| s["restartCount"].as_u64())
            .unwrap_or(0);
        if let Some(before) = self.restarts {
            if restarts > before && !self.stopped() {
                (self.emit)(StreamEvent::Notice { message: format!("Conteneur redémarré ({restarts} redémarrage(s))") });
            }
        }
        self.restarts = Some(restarts);
        Ok(true)
    }

    async fn replacements(&self) -> Result<Vec<String>, SourceError> {
        let Some(workload) = &self.workload else { return Ok(vec![]) };
        let list = rancher::kube_get(&self.target, &self.r.cluster_id, &format!("/api/v1/namespaces/{}/pods", enc(&self.r.namespace))).await?;
        let mut names: Vec<String> = items(&list)
            .iter()
            .filter(|p| p["metadata"]["deletionTimestamp"].is_null() && &workload_of(p) == workload)
            .filter_map(|p| p["metadata"]["name"].as_str().map(str::to_string))
            .collect();
        names.sort();
        Ok(names)
    }
}

fn short(e: &reqwest::Error) -> String {
    let mut source: Option<&dyn std::error::Error> = Some(e);
    let mut last = e.to_string();
    while let Some(s) = source {
        last = s.to_string();
        source = s.source();
    }
    last
}
