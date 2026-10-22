//! Lecture d'un topic **sans groupe de consommateurs ni commit** : les partitions sont lues
//! directement (ListOffsets + Fetch), sans impact sur les applications. Départ : N derniers
//! messages, « depuis », ou tout le topic ; puis suivi en direct.

use super::{Emit, StreamEvent, StreamOptions, StreamState, Timing};
use crate::config::KafkaConnection;
use crate::error::{ErrorCode, SourceError};
use crate::kafka::client::{Client, KafkaError, RawMessage, decode_records, to_source_error};
use crate::kafka::decode::{Decoder, SchemaRegistry};
use crate::logs::{KafkaMeta, LogLine};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct Partition {
    index: i32,
    leader: i32,
    /// Prochain offset à lire.
    next: i64,
}

struct Run {
    connection: KafkaConnection,
    client: Client,
    topic: String,
    topic_id: uuid::Uuid,
    options: StreamOptions,
    emit: Emit,
    timing: Timing,
    cancel: CancellationToken,
    decoder: Decoder,
    partitions: Vec<Partition>,
    ended: bool,
}

pub fn spawn(connection: KafkaConnection, secret: String, topic: String, options: StreamOptions, emit: Emit, timing: Timing, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
    let registry = connection.registry.as_ref().map(|r| Arc::new(SchemaRegistry::new(&r.url, &r.tls)));
    let client = crate::kafka::client(&connection, &secret);
    let run = Run {
        connection, client, topic, topic_id: uuid::Uuid::nil(), options, emit, timing, cancel,
        decoder: Decoder::new(registry), partitions: vec![], ended: false,
    };
    tokio::spawn(run.main())
}

fn compare(a: &RawMessage, b: &RawMessage) -> std::cmp::Ordering {
    a.timestamp.cmp(&b.timestamp).then(a.partition.cmp(&b.partition)).then(a.offset.cmp(&b.offset))
}

/// Fusion chronologique de partitions lues par lots : un message ne part que lorsque chaque
/// partition encore en cours a, en attente, un message au moins aussi récent (dans une
/// partition, les messages arrivent dans l'ordre des offsets).
#[derive(Default)]
struct Merge {
    queues: HashMap<i32, std::collections::VecDeque<RawMessage>>,
}

impl Merge {
    fn push(&mut self, messages: Vec<RawMessage>) {
        for m in messages { self.queues.entry(m.partition).or_default().push_back(m); }
    }

    /// Messages envoyables sans risque d'en voir arriver un plus ancien ; `open` : partitions
    /// dont la lecture n'est pas terminée.
    fn ready(&mut self, open: &[i32]) -> Vec<RawMessage> {
        let mut out = vec![];
        loop {
            if open.iter().any(|p| self.queues.get(p).is_none_or(|q| q.is_empty())) { break; }
            let Some(p) = self.queues.iter().filter_map(|(p, q)| q.front().map(|m| (*p, m))).min_by(|a, b| compare(a.1, b.1)).map(|(p, _)| p) else { break };
            out.push(self.queues.get_mut(&p).unwrap().pop_front().unwrap());
        }
        out
    }

    /// Fin de lecture : tout ce qui reste.
    fn rest(self) -> Vec<RawMessage> { self.queues.into_values().flatten().collect() }
}

impl Run {
    fn err(&self, e: KafkaError) -> SourceError { to_source_error(&e, &self.connection.brokers) }

    async fn main(mut self) {
        self.status(StreamState::Connecting, None);
        let result = async {
            self.load_partitions(true).await?;
            self.status(StreamState::Live, if self.options.follow { None } else { Some("Chargement de l’historique…".into()) });
            self.read_history().await?;
            if !self.options.follow {
                self.finish(StreamState::Ended, "Fin de l’historique".into(), None);
                return Ok(());
            }
            self.follow().await
        }
        .await;
        if let Err(e) = result {
            if !self.cancel.is_cancelled() { self.finish(StreamState::Error, e.message.clone(), Some(e)); }
        }
    }

    /// Partitions du topic et offsets de départ (selon les options).
    async fn load_partitions(&mut self, initial: bool) -> Result<(), SourceError> {
        let meta = self.client.metadata(Some(&[self.topic.clone()])).await.map_err(|e| self.err(e))?;
        let topic = meta.topics.into_iter().find(|t| t.name == self.topic && t.error == 0 && !t.partitions.is_empty());
        let Some(topic) = topic else {
            return Err(SourceError::new(ErrorCode::Http, format!("Topic {} introuvable.", self.topic)));
        };
        self.topic_id = topic.id;
        let known: HashMap<i32, i64> = self.partitions.iter().map(|p| (p.index, p.next)).collect();
        let starts = if initial { Some(self.start_offsets(&topic.partitions).await?) } else { None };
        let mut parts = topic.partitions.clone();
        parts.sort();
        self.partitions = parts
            .into_iter()
            .map(|(index, leader)| Partition {
                index,
                leader,
                // Nouvelle partition apparue pendant le suivi : lue depuis son début.
                next: known.get(&index).copied().or_else(|| starts.as_ref().and_then(|s| s.get(&index).copied())).unwrap_or(0),
            })
            .collect();
        Ok(())
    }

    async fn start_offsets(&self, partitions: &[(i32, i32)]) -> Result<HashMap<i32, i64>, SourceError> {
        let end = self.client.list_offsets(&self.topic, partitions, -1).await.map_err(|e| self.err(e))?;
        let begin = self.client.list_offsets(&self.topic, partitions, -2).await.map_err(|e| self.err(e))?;
        if let Some(since) = self.options.since_seconds.filter(|s| *s > 0) {
            let at = crate::text::now_ms() - since as i64 * 1000;
            let by_time = self.client.list_offsets(&self.topic, partitions, at).await.map_err(|e| self.err(e))?;
            return Ok(end.iter().map(|(p, e)| (*p, by_time.get(p).copied().filter(|o| *o >= 0).unwrap_or(*e))).collect());
        }
        match self.options.tail_lines {
            None => Ok(begin),
            // N derniers par partition, puis on ne garde que les N plus récents après fusion.
            Some(tail) => Ok(end.iter().map(|(p, e)| {
                let b = begin.get(p).copied().unwrap_or(0);
                (*p, (*e - tail as i64).max(b))
            }).collect()),
        }
    }

    /// Historique jusqu'à la fin actuelle du topic, trié par date (N derniers si demandé).
    async fn read_history(&mut self) -> Result<(), SourceError> {
        let parts: Vec<(i32, i32)> = self.partitions.iter().map(|p| (p.index, p.leader)).collect();
        let end = self.client.list_offsets(&self.topic, &parts, -1).await.map_err(|e| self.err(e))?;
        let tail = self.options.tail_lines;
        let stream_all = tail.is_none() || self.options.since_seconds.is_some_and(|s| s > 0);
        let mut collected: Vec<RawMessage> = vec![];
        let progressive = stream_all && tail.is_none_or(|t| t == 0);
        let mut merge = Merge::default();
        loop {
            if self.cancel.is_cancelled() { return Ok(()); }
            let pending: Vec<usize> = (0..self.partitions.len()).filter(|i| self.partitions[*i].next < end.get(&self.partitions[*i].index).copied().unwrap_or(0)).collect();
            if pending.is_empty() { break; }
            let messages = self.fetch_once(Some(&pending), 0).await.map_err(|e| self.err(e))?;
            if progressive {
                // « Tout » : envoyé au fil de l'eau, dans l'ordre chronologique toutes partitions
                // confondues (sans tout garder en mémoire).
                merge.push(messages);
                let open: Vec<i32> = pending.iter().map(|i| self.partitions[*i].index).filter(|p| self.partitions.iter().any(|x| x.index == *p && x.next < end.get(p).copied().unwrap_or(0))).collect();
                let ready = merge.ready(&open);
                if !ready.is_empty() { self.send(ready).await; }
            } else {
                collected.extend(messages);
            }
        }
        collected.extend(merge.rest());
        collected.sort_by(compare);
        if let Some(t) = tail {
            let keep = t as usize;
            if collected.len() > keep { collected.drain(..collected.len() - keep); }
        }
        self.send(collected).await;
        Ok(())
    }

    /// Suivi : longues interrogations, reprise automatique sur erreur.
    async fn follow(&mut self) -> Result<(), SourceError> {
        let mut attempt = 0;
        while !self.cancel.is_cancelled() {
            match self.fetch_once(None, 1_000).await {
                Ok(mut messages) => {
                    attempt = 0;
                    messages.sort_by(compare);
                    self.send(messages).await;
                }
                Err(e) => {
                    if self.cancel.is_cancelled() { return Ok(()); }
                    let error = self.err(e);
                    if matches!(error.code, ErrorCode::Unauthorized | ErrorCode::Forbidden) { return Err(error); }
                    let delay = self.timing.retry_delay(attempt);
                    attempt += 1;
                    self.status(StreamState::Reconnecting, Some(format!("{} — nouvelle tentative…", error.message)));
                    tokio::select! {
                        _ = self.cancel.cancelled() => return Ok(()),
                        _ = tokio::time::sleep(delay) => {}
                    }
                    self.client.reset().await;
                    let _ = self.load_partitions(false).await;
                    self.status(StreamState::Live, None);
                }
            }
        }
        Ok(())
    }

    /// Un Fetch par broker leader ; avance les offsets des partitions lues.
    async fn fetch_once(&mut self, only: Option<&[usize]>, max_wait: i32) -> Result<Vec<RawMessage>, KafkaError> {
        let indexes: Vec<usize> = only.map(|o| o.to_vec()).unwrap_or_else(|| (0..self.partitions.len()).collect());
        let mut by_leader: HashMap<i32, Vec<usize>> = HashMap::new();
        for i in indexes { by_leader.entry(self.partitions[i].leader).or_default().push(i); }
        let groups: Vec<(i32, Vec<(i32, i64)>)> = by_leader
            .iter()
            .map(|(leader, group)| (*leader, group.iter().map(|i| (self.partitions[*i].index, self.partitions[*i].next)).collect()))
            .collect();
        let futures: Vec<_> = groups.iter().map(|(leader, parts)| self.client.fetch(*leader, &self.topic, self.topic_id, parts, max_wait)).collect();
        let results = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(vec![]),
            r = futures_util::future::join_all(futures) => r,
        };
        let mut messages = vec![];
        for result in results {
            for part in result? {
                let Some(p) = self.partitions.iter_mut().find(|p| p.index == part.partition) else { continue };
                if part.error == 1 {
                    // OFFSET_OUT_OF_RANGE (messages expirés) : reprise au début disponible.
                    p.next = part.log_start_offset;
                    continue;
                }
                if part.error != 0 { return Err(KafkaError::Code(part.error, String::new())); }
                if let Some(records) = part.records {
                    let (msgs, next) = decode_records(p.index, records, p.next);
                    if let Some(n) = next { p.next = p.next.max(n); }
                    messages.extend(msgs);
                }
            }
        }
        Ok(messages)
    }

    /// Décode et envoie par lots.
    async fn send(&self, messages: Vec<RawMessage>) {
        for chunk in messages.chunks(self.timing.batch_lines.max(1)) {
            if self.cancel.is_cancelled() || self.ended { return; }
            let mut lines = Vec::with_capacity(chunk.len());
            for m in chunk { lines.push(self.line(m).await); }
            (self.emit)(StreamEvent::Lines { lines });
        }
    }

    async fn line(&self, m: &RawMessage) -> LogLine {
        let value = self.decoder.decode(m.value.as_deref()).await;
        let key = self.decoder.decode(m.key.as_deref()).await;
        let headers = (!m.headers.is_empty()).then(|| {
            m.headers.iter().map(|(k, v)| (k.clone(), v.as_ref().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default())).collect()
        });
        LogLine {
            ts: chrono::DateTime::from_timestamp_millis(m.timestamp).map(crate::text::iso),
            text: value.text,
            meta: Some(KafkaMeta {
                partition: m.partition,
                offset: m.offset.to_string(),
                key: if key.format == "null" { None } else { Some(key.text) },
                headers,
                format: value.format.into(),
                warning: value.warning.or(key.warning),
            }),
        }
    }

    fn finish(&mut self, state: StreamState, message: String, error: Option<SourceError>) {
        if self.ended || self.cancel.is_cancelled() { return; }
        self.ended = true;
        (self.emit)(StreamEvent::Status { state, message: Some(message), error, replacements: None });
    }

    fn status(&self, state: StreamState, message: Option<String>) {
        if self.ended || self.cancel.is_cancelled() { return; }
        (self.emit)(StreamEvent::status(state, message));
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn m(partition: i32, offset: i64, timestamp: i64) -> RawMessage {
        RawMessage { partition, offset, timestamp, key: None, value: None, headers: vec![] }
    }

    #[test]
    fn fusion_chronologique_par_lots() {
        let mut merge = Merge::default();
        // Lot 1 : la partition 1 n'a encore rien donné : rien ne part.
        merge.push(vec![m(0, 0, 10), m(0, 1, 30)]);
        assert!(merge.ready(&[0, 1]).is_empty());
        // Lot 2 : la partition 1 arrive ; part tout ce qui précède sa tête connue.
        merge.push(vec![m(1, 0, 20), m(1, 1, 40)]);
        let out: Vec<i64> = merge.ready(&[0, 1]).iter().map(|x| x.timestamp).collect();
        assert_eq!(out, vec![10, 20, 30]);
        // La partition 0 est terminée : la 1 se vide seule.
        let out: Vec<i64> = merge.ready(&[1]).iter().map(|x| x.timestamp).collect();
        assert_eq!(out, vec![40]);
        merge.push(vec![m(1, 2, 50)]);
        assert_eq!(merge.rest().len(), 1);
    }
}
