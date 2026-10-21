//! Kafka : test de connexion, liste des topics, lecture (voir `streams::kafka`).

pub mod client;
pub mod decode;
pub mod scram;

use crate::config::KafkaConnection;
use crate::error::SourceError;
use crate::rancher::SourceNode;
use client::{Client, Options};

pub fn client(connection: &KafkaConnection, secret: &str) -> Client {
    Client::new(Options::of(connection, secret))
}

fn err(connection: &KafkaConnection) -> impl Fn(client::KafkaError) -> SourceError + '_ {
    move |e| client::to_source_error(&e, &connection.brokers)
}

/// Test : brokers et topics visibles, puis Schema Registry s'il est configuré.
pub async fn test(connection: &KafkaConnection, secret: &str) -> Result<String, SourceError> {
    let c = client(connection, secret);
    let meta = c.metadata(None).await.map_err(err(connection))?;
    let topics = meta.topics.iter().filter(|t| !t.internal).count();
    let mut summary = format!("{} broker(s), {topics} topic(s)", meta.brokers.len());
    if let Some(r) = &connection.registry {
        let subjects = decode::SchemaRegistry::new(&r.url, &r.tls).check().await?;
        summary.push_str(&format!(", Schema Registry OK ({subjects} sujet(s))"));
    }
    Ok(summary)
}

/// Topics (hors internes `__…`), triés, avec leur nombre de partitions.
pub async fn list_topics(connection: &KafkaConnection, secret: &str) -> Result<Vec<SourceNode>, SourceError> {
    let meta = client(connection, secret).metadata(None).await.map_err(err(connection))?;
    let mut topics: Vec<SourceNode> = meta
        .topics
        .into_iter()
        .filter(|t| !t.name.starts_with("__") && !t.internal)
        .map(|t| SourceNode { id: t.name, kind: "topic", expandable: false, partitions: Some(t.partitions.len() as u64), ..Default::default() })
        .collect();
    topics.sort_by(|a, b| crate::text::locale_cmp(&a.id, &b.id));
    Ok(topics)
}
