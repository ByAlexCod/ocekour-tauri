//! Point d'entrée unique vers les sources (Rancher, Kafka, S3), quel que soit leur type.

use crate::config::Connection;
use crate::connections::Connections;
use crate::error::SourceError;
use crate::rancher::SourceNode;
use serde_json::Value;

/// Enfants d'un nœud : Rancher [] → clusters, [cluster] → namespaces… ; Kafka [] → topics ;
/// S3 [] → buckets. Connexion et secret relus à chaque appel : toujours ceux du moment.
pub async fn browse(connections: &Connections, connection_id: &str, path: &[String]) -> Result<Vec<SourceNode>, SourceError> {
    if path.len() > 4 || path.iter().any(|p| p.is_empty()) {
        return Err(SourceError::invalid("Chemin de source invalide."));
    }
    if connection_id == crate::files::LOCAL { return crate::files::browse(path); }
    match connections.get(connection_id)? {
        Connection::Rancher(_) => crate::rancher::browse(&connections.target(connection_id)?, path).await,
        Connection::Kafka(k) => {
            if !path.is_empty() { return Ok(vec![]); }
            crate::kafka::list_topics(&k, &connections.secret(connection_id)?).await
        }
        Connection::S3(s) => {
            if !path.is_empty() { return Ok(vec![]); }
            crate::s3::list_buckets(&s, &connections.secret(connection_id)?).await
        }
        Connection::Sonde(s) => crate::sonde::client::browse(&s, &connections.secret(connection_id)?, path).await,
    }
}

/// Variante IPC : `{ connectionId, path }`.
pub async fn browse_value(connections: &Connections, input: &Value) -> Result<Vec<SourceNode>, SourceError> {
    let id = input["connectionId"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| SourceError::invalid("connectionId attendu"))?;
    let path: Vec<String> = match &input["path"] {
        Value::Array(a) => a.iter().map(|p| p.as_str().map(str::to_string).ok_or_else(|| SourceError::invalid("Chemin de source invalide."))).collect::<Result<_, _>>()?,
        Value::Null => vec![],
        _ => return Err(SourceError::invalid("Chemin de source invalide.")),
    };
    browse(connections, id, &path).await
}
