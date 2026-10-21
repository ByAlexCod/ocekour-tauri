//! Lien de partage `ocekour://open?d=<base64url JSON>` : de quoi rouvrir la même vue chez un
//! collègue. JAMAIS de secret ni d'id local : la connexion est retrouvée par son URL
//! (Rancher, S3) ou ses brokers (Kafka). Même format que l'interface.

use crate::config::Connection;
use crate::schema::{self as z, S};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

const PREFIX: &str = "ocekour://open?d=";

fn options() -> S {
    z::object(vec![
        ("follow", z::boolean()),
        ("tailLines", z::int().min(0).optional()),
        ("sinceSeconds", z::int().min(1).optional()),
        ("previous", z::boolean().optional()),
    ])
}

fn filters() -> S {
    z::object(vec![
        ("text", z::string().max(2_000)),
        ("caseSensitive", z::boolean()),
        ("regex", z::boolean()),
        ("mode", z::enumeration(&["filter", "highlight"])),
        ("levels", z::array(z::enumeration(&["trace", "debug", "info", "warn", "error", "fatal", "unknown"])).optional()),
    ])
}

fn name() -> S { z::string().min(1).max(512) }

pub fn link_schema() -> S {
    z::disc("t", vec![
        (
            "rancher",
            z::object(vec![
                ("v", z::literal(json!(1))),
                ("t", z::literal(json!("rancher"))),
                ("url", z::string().url()),
                ("clusterId", name()),
                ("clusterName", name()),
                ("namespace", name()),
                ("pod", name()),
                ("container", name()),
                ("workload", z::string().max(512)),
                ("options", options()),
                ("filters", filters().optional()),
            ]),
        ),
        (
            "kafka",
            z::object(vec![
                ("v", z::literal(json!(1))),
                ("t", z::literal(json!("kafka"))),
                ("brokers", z::array(name()).min(1).max(50)),
                ("topic", name()),
                ("options", options()),
                ("filters", filters().optional()),
            ]),
        ),
        (
            "s3",
            z::object(vec![
                ("v", z::literal(json!(1))),
                ("t", z::literal(json!("s3"))),
                ("url", z::string().url()),
                ("bucket", name()),
                ("prefix", z::string().max(1024)),
            ]),
        ),
    ])
}

pub fn encode(link: &Value) -> String {
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(crate::logs::json_text(link)))
}

/// Premier lien Ocekour contenu dans un texte.
pub fn find(text: &str) -> Option<&str> {
    static RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"ocekour://open\?d=[A-Za-z0-9_-]+").unwrap());
    RE.find(text).map(|m| m.as_str())
}

/// Lien collé → contenu validé ; erreur lisible sinon.
pub fn decode(text: &str) -> anyhow::Result<Value> {
    let link = find(text.trim()).ok_or_else(|| anyhow::anyhow!("Ce texte n'est pas un lien Ocekour (ocekour://open?d=…)."))?;
    let json: Value = URL_SAFE_NO_PAD
        .decode(link[PREFIX.len()..].trim_end_matches('='))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| anyhow::anyhow!("Lien Ocekour illisible (copié en entier ?)."))?;
    if json["v"] != json!(1) {
        anyhow::bail!("Lien créé par une autre version d’Ocekour : mettez l’application à jour.");
    }
    link_schema().parse(&json).map_err(|_| anyhow::anyhow!("Lien Ocekour invalide (contenu incomplet)."))
}

fn normalize_url(url: &str) -> String { url.trim().trim_end_matches('/').to_lowercase() }

/// Connexion locale qui correspond au lien (même URL, ou un broker en commun).
pub fn find_connection<'a>(link: &Value, connections: &'a [Connection]) -> Option<&'a Connection> {
    match link["t"].as_str()? {
        "kafka" => {
            let brokers: Vec<String> = link["brokers"].as_array()?.iter().filter_map(|b| b.as_str()).map(|b| b.trim().to_lowercase()).collect();
            connections.iter().find(|c| match c {
                Connection::Kafka(k) => k.brokers.iter().any(|b| brokers.contains(&b.trim().to_lowercase())),
                _ => false,
            })
        }
        t => {
            let url = normalize_url(link["url"].as_str()?);
            connections.iter().find(|c| match (t, c) {
                ("rancher", Connection::Rancher(r)) => normalize_url(&r.url) == url,
                ("s3", Connection::S3(s)) => normalize_url(&s.url) == url,
                _ => false,
            })
        }
    }
}

/// Lien d'une vue : conteneur Rancher, topic Kafka ou dossier S3.
pub fn link_for_container(
    connection_url: &str,
    cluster_id: &str,
    cluster_name: &str,
    namespace: &str,
    pod: &str,
    container: &str,
    workload: &str,
    options: Value,
    filters: Option<Value>,
) -> Value {
    let mut link = json!({
        "v": 1, "t": "rancher", "url": connection_url, "clusterId": cluster_id, "clusterName": cluster_name,
        "namespace": namespace, "pod": pod, "container": container, "workload": workload, "options": options,
    });
    if let Some(f) = filters { link["filters"] = f; }
    link
}

pub fn link_for_topic(brokers: &[String], topic: &str, options: Value, filters: Option<Value>) -> Value {
    let mut link = json!({ "v": 1, "t": "kafka", "brokers": brokers, "topic": topic, "options": options });
    if let Some(f) = filters { link["filters"] = f; }
    link
}

pub fn link_for_bucket(url: &str, bucket: &str, prefix: &str) -> Value {
    json!({ "v": 1, "t": "s3", "url": url, "bucket": bucket, "prefix": prefix })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conns() -> Vec<Connection> {
        let schema = crate::config::connection_schema();
        [
            json!({ "id": "rancher-1", "type": "rancher", "name": "Prod", "url": "https://RANCHER.intra/", "clusters": [{ "id": "c-1", "name": "prod" }] }),
            json!({ "id": "kafka-1", "type": "kafka", "name": "Kafka", "brokers": ["k1:9092", "k2:9092"], "security": { "username": "moi" } }),
        ]
        .iter()
        .map(|v| schema.parse_into(v).unwrap())
        .collect()
    }

    #[test]
    fn aller_retour() {
        let link = link_for_container("https://rancher.intra", "c-1", "prod", "app", "api-7d9f-x2k4p", "api", "api", json!({ "follow": true, "tailLines": 1000 }), Some(json!({ "text": "paiement é", "caseSensitive": false, "regex": false, "mode": "highlight", "levels": ["warn", "error"] })));
        let text = encode(&link);
        assert!(regex::Regex::new(r"^ocekour://open\?d=[A-Za-z0-9_-]+$").unwrap().is_match(&text));
        let decoded = decode(&format!("Regarde ça : {text} merci")).unwrap();
        assert_eq!(decoded, link);
        let c = conns();
        assert_eq!(find_connection(&decoded, &c).unwrap().id(), "rancher-1");
    }

    #[test]
    fn compatible_avec_l_interface() {
        // Produit par share-link.ts (encodeLink) : relu à l'identique.
        let from_ts = "ocekour://open?d=eyJ2IjoxLCJ0Ijoia2Fma2EiLCJicm9rZXJzIjpbIksyOjkwOTIiXSwidG9waWMiOiJ0Iiwib3B0aW9ucyI6eyJmb2xsb3ciOnRydWV9fQ";
        let link = decode(from_ts).unwrap();
        assert_eq!(link, json!({ "v": 1, "t": "kafka", "brokers": ["K2:9092"], "topic": "t", "options": { "follow": true } }));
        assert_eq!(encode(&link), from_ts);
        assert_eq!(find_connection(&link, &conns()).unwrap().id(), "kafka-1");
    }

    #[test]
    fn liens_invalides() {
        assert!(decode("bonjour").unwrap_err().to_string().contains("pas un lien Ocekour"));
        assert!(decode("ocekour://open?d=@@").unwrap_err().to_string().contains("pas un lien"));
        assert!(decode("ocekour://open?d=abc").unwrap_err().to_string().contains("illisible"));
        let future = URL_SAFE_NO_PAD.encode(r#"{"v":2}"#);
        assert!(decode(&format!("ocekour://open?d={future}")).unwrap_err().to_string().contains("autre version"));
        let partial = URL_SAFE_NO_PAD.encode(r#"{"v":1,"t":"s3"}"#);
        assert!(decode(&format!("ocekour://open?d={partial}")).unwrap_err().to_string().contains("incomplet"));
        assert!(find("rien").is_none());
    }
}
