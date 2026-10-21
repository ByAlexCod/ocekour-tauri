//! Préconfiguration portée par une clé d'activation : connexions, profils et URL des services
//! au format de config.yaml, ajoutés à l'activation. JAMAIS de secret.

use crate::config::{Connection, Profile, connection_schema, profile_schema, service_schema, SERVICE_IDS};
use serde_json::{Map, Value};
use sha2::Digest;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preset {
    pub connections: Vec<Connection>,
    pub profiles: Vec<Profile>,
    /// Services (id → réglages validés, JSON).
    pub services: Map<String, Value>,
}

static SECRET_FIELD: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"(?i)secret|token|password|passwd|mot.?de.?passe|pwd").unwrap());

/// Identifiant stable d'une connexion préconfigurée.
pub fn connection_id(kind: &str, name: &str) -> String {
    format!("{kind}-{}", &hex::encode(sha2::Sha256::digest(format!("{kind}:{name}")))[..8])
}

fn find_secrets(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::Array(items) => items.iter().enumerate().find_map(|(i, v)| find_secrets(v, &format!("{path}[{i}]"))),
        Value::Object(map) => map.iter().find_map(|(k, v)| {
            if SECRET_FIELD.is_match(k) { Some(format!("{path}.{k}")) } else { find_secrets(v, &format!("{path}.{k}")) }
        }),
        _ => None,
    }
}

fn describe(issues: &[crate::schema::Issue], at: &str) -> anyhow::Error {
    let i = &issues[0];
    anyhow::anyhow!("{at}.{} : {}", i.path_string(), i.message)
}

/// Valide une préconfiguration ; erreur lisible (avec le chemin fautif) sinon.
pub fn validate(raw: Option<&Value>) -> anyhow::Result<Preset> {
    let Some(raw) = raw.filter(|v| !v.is_null()) else { return Ok(Preset::default()) };
    let Some(obj) = raw.as_object() else { anyhow::bail!("Préconfiguration : objet attendu (connections, profiles).") };
    if let Some(k) = obj.keys().find(|k| !["connections", "profiles", "services"].contains(&k.as_str())) {
        anyhow::bail!("Préconfiguration : « {k} » inconnu (seuls connections, profiles et services).");
    }
    if let Some(s) = find_secrets(raw, "preconfiguration") {
        anyhow::bail!("{s} : pas de secret dans une clé (chacun saisit les siens).");
    }
    let empty = Value::Array(vec![]);
    let (connections, profiles) = (obj.get("connections").unwrap_or(&empty), obj.get("profiles").unwrap_or(&empty));
    let (Some(connections), Some(profiles)) = (connections.as_array(), profiles.as_array()) else {
        anyhow::bail!("Préconfiguration : connections et profiles sont des listes.");
    };
    let mut parsed: Vec<Connection> = vec![];
    for (i, c) in connections.iter().enumerate() {
        let mut entry = c.as_object().cloned().unwrap_or_default();
        if !entry.get("id").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()) {
            let kind = entry.get("type").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            entry.insert("id".into(), Value::String(connection_id(&kind, &name)));
        }
        let c: Connection = connection_schema().parse_into(&Value::Object(entry)).map_err(|e| describe(&e, &format!("connections[{i}]")))?;
        if parsed.iter().any(|p| p.id() == c.id()) {
            anyhow::bail!("connections : « {} » en double (même id {}).", c.name(), c.id());
        }
        parsed.push(c);
    }
    let profiles = profiles
        .iter()
        .enumerate()
        .map(|(i, p)| profile_schema().parse_into::<Profile>(p).map_err(|e| describe(&e, &format!("profiles[{i}]"))))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let services = match obj.get("services") {
        None => Map::new(),
        Some(Value::Object(s)) => {
            let mut out = Map::new();
            for (id, value) in s {
                if !SERVICE_IDS.contains(&id.as_str()) {
                    anyhow::bail!("services.{id} : service inconnu ({}).", SERVICE_IDS.join(", "));
                }
                if let Some(extra) = value.as_object().and_then(|o| o.keys().find(|k| *k != "url" && *k != "tls")) {
                    anyhow::bail!("services.{id}.{extra} : seuls url et tls sont acceptés.");
                }
                out.insert(id.clone(), service_schema().parse(value).map_err(|e| describe(&e, &format!("services.{id}")))?);
            }
            out
        }
        Some(_) => anyhow::bail!("Préconfiguration : services est un objet (gitlab, jira, confluence, rocketchat)."),
    };
    Ok(Preset { connections: parsed, profiles, services })
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim().trim_end_matches('/').to_lowercase() == b.trim().trim_end_matches('/').to_lowercase()
}

/// Connexion déjà présente (même id, ou même serveur : URL ou broker commun) ?
fn already_there<'a>(preset: &Connection, existing: impl Iterator<Item = &'a Connection>) -> bool {
    existing.into_iter().any(|c| {
        if c.id() == preset.id() { return true; }
        match (c, preset) {
            (Connection::Kafka(a), Connection::Kafka(b)) => b.brokers.iter().any(|x| a.brokers.iter().any(|y| y.to_lowercase() == x.to_lowercase())),
            (Connection::Rancher(a), Connection::Rancher(b)) => same_url(&a.url, &b.url),
            (Connection::S3(a), Connection::S3(b)) => same_url(&a.url, &b.url),
            _ => false,
        }
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub connections: Vec<Connection>,
    pub profiles: Vec<Profile>,
    pub services: Map<String, Value>,
    pub skipped: Vec<String>,
}

/// Ce qu'il faut ajouter : les connexions absentes, les profils dont le `match` n'existe pas,
/// les services non configurés (la configuration de l'utilisateur prime).
pub fn plan(preset: &Preset, connections: &[Connection], profiles: &[Profile], services: &crate::config::Services) -> Plan {
    let mut out = Plan::default();
    for c in &preset.connections {
        if already_there(c, connections.iter().chain(out.connections.iter())) { out.skipped.push(c.name().into()); } else { out.connections.push(c.clone()); }
    }
    out.profiles = preset.profiles.iter().filter(|p| !profiles.iter().any(|q| q.match_ == p.match_)).cloned().collect();
    for (id, s) in &preset.services {
        if services.get(id).is_none() { out.services.insert(id.clone(), s.clone()); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw() -> Value {
        json!({
            "connections": [
                { "type": "rancher", "name": "Rancher prod", "url": "https://rancher.intra", "clusters": [{ "id": "c-1", "name": "prod" }] },
                { "type": "kafka", "name": "Kafka prod", "brokers": ["k1:9092", "k2:9092"] },
                { "id": "s3-perso", "type": "s3", "name": "S3", "url": "https://s3.intra", "accessKeyId": "AK" },
            ],
            "profiles": [{ "match": "prod/*/api-*", "columns": ["traceId"] }],
        })
    }

    #[test]
    fn valide_avec_id_stable() {
        let p = validate(Some(&raw())).unwrap();
        assert_eq!(p.connections.iter().map(|c| c.id().to_string()).collect::<Vec<_>>(), vec![connection_id("rancher", "Rancher prod"), connection_id("kafka", "Kafka prod"), "s3-perso".into()]);
        assert!(regex::Regex::new("^rancher-[0-9a-f]{8}$").unwrap().is_match(&connection_id("rancher", "Rancher prod")));
        match &p.connections[1] {
            Connection::Kafka(k) => assert_eq!((k.security.protocol.as_str(), k.security.mechanism.as_str()), ("SASL_PLAINTEXT", "SCRAM-SHA-512")),
            _ => panic!(),
        }
        assert_eq!(p.profiles[0].columns, vec!["traceId"]);
        assert_eq!(validate(None).unwrap(), Preset::default());
    }

    /// Même identifiant que Ocekour 1.x (sha256 Node).
    #[test]
    fn id_compatible() {
        assert_eq!(connection_id("kafka", "Kafka prod"), format!("kafka-{}", &hex::encode(sha2::Sha256::digest("kafka:Kafka prod"))[..8]));
    }

    #[test]
    fn refuse_secrets_cles_inconnues_invalides_doublons() {
        let r = raw();
        let mut k = r["connections"][1].clone();
        k["security"] = json!({ "password": "x" });
        assert!(validate(Some(&json!({ "connections": [k] }))).unwrap_err().to_string().contains("security.password : pas de secret"));
        assert!(validate(Some(&json!({ "connections": [{ "type": "s3", "name": "S", "secretAccessKey": "x" }] }))).unwrap_err().to_string().contains("secretAccessKey"));
        assert!(validate(Some(&json!({ "token": "x" }))).unwrap_err().to_string().contains("token"));
        assert!(validate(Some(&json!({ "theme": "dark" }))).unwrap_err().to_string().contains("« theme » inconnu"));
        assert!(validate(Some(&json!({ "connections": [{ "type": "rancher", "name": "R" }] }))).unwrap_err().to_string().contains("connections[0].url"));
        let dup = json!({ "connections": [r["connections"][1], r["connections"][1]] });
        assert!(validate(Some(&dup)).unwrap_err().to_string().contains("en double"));
    }

    #[test]
    fn n_ajoute_que_ce_qui_manque() {
        let p = validate(Some(&raw())).unwrap();
        let schema = connection_schema();
        let mine: Vec<Connection> = vec![
            schema.parse_into(&json!({ "id": "rancher-a1", "type": "rancher", "name": "Mon Rancher", "url": "https://RANCHER.intra/" })).unwrap(),
            schema.parse_into(&json!({ "id": "kafka-a2", "type": "kafka", "name": "K", "brokers": ["k2:9092"] })).unwrap(),
        ];
        let existing: Vec<Profile> = vec![profile_schema().parse_into(&json!({ "match": "prod/*/api-*" })).unwrap()];
        let plan1 = plan(&p, &mine, &existing, &Default::default());
        assert_eq!(plan1.connections.iter().map(|c| c.name()).collect::<Vec<_>>(), vec!["S3"]);
        assert_eq!(plan1.skipped, vec!["Rancher prod", "Kafka prod"]);
        assert!(plan1.profiles.is_empty());
        let again = plan(&p, &p.connections, &p.profiles, &Default::default());
        assert!(again.connections.is_empty());
    }
}
