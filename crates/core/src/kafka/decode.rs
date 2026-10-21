//! Décodage des clés et valeurs : format Confluent (octet magique 0 + id de schéma) via le
//! Schema Registry (Avro, JSON) ; sinon JSON ou texte UTF-8 ; sinon hexadécimal.

use crate::config::Tls;
use crate::error::{ErrorCode, SourceError};
use crate::net;
use apache_avro::types::Value as Avro;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Schema Registry (API Confluent), sans authentification ; schémas mis en cache.
pub struct SchemaRegistry {
    url: String,
    tls: Tls,
    cache: Mutex<HashMap<i32, (String, String)>>,
}

impl SchemaRegistry {
    pub fn new(url: &str, tls: &Tls) -> Self {
        SchemaRegistry { url: url.into(), tls: tls.clone(), cache: Mutex::new(HashMap::new()) }
    }

    async fn get(&self, path: &str) -> Result<Value, SourceError> {
        let url = crate::rancher::join(&self.url, path)?;
        let host = net::host_of(url.as_str());
        let client = net::http_client(&self.tls)?;
        let res = client
            .get(url)
            .header("Accept", "application/vnd.schemaregistry.v1+json, application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| {
                let detail = if e.is_timeout() { format!("Schema Registry {host} : délai dépassé") } else { net::from_reqwest(&e, &self.url).message };
                SourceError::new(ErrorCode::Network, format!("Schema Registry injoignable ({host}) : {detail}"))
            })?;
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        if status >= 400 {
            let fallback = format!("Schema Registry : réponse {status} sur /{path}");
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["message"].as_str().map(|m| format!("Schema Registry : {m}")))
                .unwrap_or(fallback);
            return Err(SourceError::new(ErrorCode::Http, message));
        }
        serde_json::from_str(&body)
            .map_err(|_| SourceError::new(ErrorCode::Http, format!("{} ne répond pas comme un Schema Registry.", self.url)))
    }

    /// Texte du schéma et type (`AVRO` par défaut).
    pub async fn schema(&self, id: i32) -> Result<(String, String), SourceError> {
        if let Some(s) = self.cache.lock().await.get(&id) { return Ok(s.clone()); }
        let r = self.get(&format!("schemas/ids/{id}")).await?;
        let schema = r["schema"].as_str().unwrap_or_default().to_string();
        let kind = r["schemaType"].as_str().unwrap_or("AVRO").to_string();
        self.cache.lock().await.insert(id, (schema.clone(), kind.clone()));
        Ok((schema, kind))
    }

    /// Nombre de sujets (test de connexion).
    pub async fn check(&self) -> Result<usize, SourceError> {
        Ok(self.get("subjects").await?.as_array().map(|a| a.len()).unwrap_or(0))
    }
}

/// Résultat du décodage d'une clé ou d'une valeur.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub text: String,
    /// `avro`, `json`, `text`, `hex` ou `null`.
    pub format: &'static str,
    pub warning: Option<String>,
}

enum Kind {
    Avro(apache_avro::Schema),
    Json,
}

pub struct Decoder {
    registry: Option<Arc<SchemaRegistry>>,
    types: Mutex<HashMap<i32, Arc<Kind>>>,
}

impl Decoder {
    pub fn new(registry: Option<Arc<SchemaRegistry>>) -> Self {
        Decoder { registry, types: Mutex::new(HashMap::new()) }
    }

    async fn kind(&self, id: i32) -> Result<Arc<Kind>, String> {
        if let Some(k) = self.types.lock().await.get(&id) { return Ok(k.clone()); }
        let (schema, kind) = self.registry.as_ref().unwrap().schema(id).await.map_err(|e| e.message)?;
        let k = Arc::new(match kind.as_str() {
            "JSON" => Kind::Json,
            "AVRO" => Kind::Avro(apache_avro::Schema::parse_str(&schema).map_err(|e| e.to_string())?),
            other => return Err(format!("format {other} non pris en charge")),
        });
        self.types.lock().await.insert(id, k.clone());
        Ok(k)
    }

    pub async fn decode(&self, data: Option<&[u8]>) -> Decoded {
        let Some(buffer) = data else { return Decoded { text: String::new(), format: "null", warning: None } };
        if buffer.len() > 5 && buffer[0] == 0 {
            let id = i32::from_be_bytes(buffer[1..5].try_into().unwrap());
            if self.registry.is_none() {
                return Decoded {
                    warning: Some(format!("Message au format Schema Registry (schéma {id}) : aucun registre configuré.")),
                    ..plain(buffer)
                };
            }
            let payload = &buffer[5..];
            let result = match self.kind(id).await {
                Err(e) => Err(e),
                Ok(k) => match k.as_ref() {
                    Kind::Json => Ok(Decoded { text: String::from_utf8_lossy(payload).into_owned(), format: "json", warning: None }),
                    Kind::Avro(schema) => {
                        let mut reader = payload;
                        #[allow(deprecated)]
                        apache_avro::from_avro_datum(schema, &mut reader, None)
                            .map(|v| Decoded { text: crate::logs::json_text(&avro_to_json(v)), format: "avro", warning: None })
                            .map_err(|e| e.to_string())
                    }
                },
            };
            return result.unwrap_or_else(|e| Decoded { warning: Some(format!("Décodage impossible (schéma {id}) : {e}")), ..plain(buffer) });
        }
        plain(buffer)
    }
}

/// Sans schéma : texte UTF-8 (JSON détecté) ou hexadécimal.
fn plain(buffer: &[u8]) -> Decoded {
    if let Ok(text) = std::str::from_utf8(buffer) {
        // Caractères de contrôle (hors \t \n \r) : binaire déguisé, on préfère l'hexadécimal.
        if !text.chars().any(|c| matches!(c as u32, 0..=8 | 0x0e..=0x1f)) {
            let t = text.trim();
            let json = (t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<Value>(t).is_ok();
            return Decoded { text: text.into(), format: if json { "json" } else { "text" }, warning: None };
        }
    }
    Decoded { text: hex::encode(buffer), format: "hex", warning: None }
}

/// Valeur Avro → JSON (unions dépliées, octets en hexadécimal).
pub fn avro_to_json(v: Avro) -> Value {
    match v {
        Avro::Null => Value::Null,
        Avro::Boolean(b) => json!(b),
        Avro::Int(i) | Avro::Date(i) | Avro::TimeMillis(i) => json!(i),
        Avro::Long(i) | Avro::TimeMicros(i) | Avro::TimestampMillis(i) | Avro::TimestampMicros(i) | Avro::TimestampNanos(i)
        | Avro::LocalTimestampMillis(i) | Avro::LocalTimestampMicros(i) | Avro::LocalTimestampNanos(i) => json!(i),
        Avro::Float(f) => serde_json::Number::from_f64(f as f64).map(Value::Number).unwrap_or(Value::Null),
        Avro::Double(f) => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null),
        Avro::Bytes(b) | Avro::Fixed(_, b) => json!(hex::encode(b)),
        Avro::String(s) => json!(s),
        Avro::Uuid(u) => json!(u.to_string()),
        Avro::Enum(_, s) => json!(s),
        Avro::Union(_, inner) => avro_to_json(*inner),
        Avro::Array(items) => Value::Array(items.into_iter().map(avro_to_json).collect()),
        Avro::Map(m) => Value::Object(m.into_iter().map(|(k, v)| (k, avro_to_json(v))).collect()),
        Avro::Record(fields) => Value::Object(fields.into_iter().map(|(k, v)| (k, avro_to_json(v))).collect::<Map<_, _>>()),
        Avro::Decimal(d) => json!(hex::encode(<Vec<u8>>::try_from(d).unwrap_or_default())),
        Avro::BigDecimal(d) => json!(d.to_string()),
        Avro::Duration(d) => json!(format!("{d:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn formats_sans_registre() {
        let d = Decoder::new(None);
        assert_eq!(d.decode(None).await.format, "null");
        assert_eq!(d.decode(Some(b"texte simple")).await, Decoded { text: "texte simple".into(), format: "text", warning: None });
        assert_eq!(d.decode(Some(br#"{"a":1}"#)).await.format, "json");
        assert_eq!(d.decode(Some(&[0xff, 0x00, 0x10])).await, Decoded { text: "ff0010".into(), format: "hex", warning: None });
        let r = d.decode(Some(&[0, 0, 0, 0, 7, 1, 2])).await;
        assert_eq!(r.format, "hex");
        assert!(r.warning.unwrap().contains("schéma 7"));
    }

    #[test]
    fn avro_en_json() {
        let schema = apache_avro::Schema::parse_str(
            r#"{"type":"record","name":"Commande","fields":[{"name":"id","type":"long"},{"name":"client","type":"string"},{"name":"note","type":["null","string"],"default":null}]}"#,
        )
        .unwrap();
        let mut record = apache_avro::types::Record::new(&schema).unwrap();
        record.put("id", 42i64);
        record.put("client", "Dupont");
        record.put("note", Avro::Union(0, Box::new(Avro::Null)));
        #[allow(deprecated)]
        let bytes = apache_avro::to_avro_datum(&schema, record).unwrap();
        #[allow(deprecated)]
        let v = apache_avro::from_avro_datum(&schema, &mut bytes.as_slice(), None).unwrap();
        assert_eq!(crate::logs::json_text(&avro_to_json(v)), r#"{"id":42,"client":"Dupont","note":null}"#);
    }
}
