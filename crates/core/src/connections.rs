//! Connexions (Rancher, Kafka, S3) : config.yaml pour le paramétrage, coffre pour les secrets.

use crate::analytics::{AnalyticsEvent, track};
use crate::config::{ConfigStore, Connection, connection_schema};
use crate::error::{ErrorCode, SourceError};
use crate::licence::preset::{self, Preset};
use crate::rancher::{RancherCluster, RancherDraft, RancherTarget};
use crate::secrets::SecretStore;
use crate::ypath;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::sync::Arc;

fn field_label(field: &str) -> Option<&'static str> {
    Some(match field {
        "name" => "Nom",
        "url" => "URL",
        "brokers" => "Brokers",
        "accessKeyId" => "Clé d'accès",
        "registry" => "Schema Registry",
        _ => return None,
    })
}

fn missing_secret(kind: &str) -> &'static str {
    match kind {
        "rancher" => "Aucun token enregistré",
        "kafka" => "Aucun mot de passe enregistré",
        "s3" => "Aucune clé secrète enregistrée",
        "sonde" => "Aucun jeton de sonde enregistré",
        _ => "Aucun secret enregistré",
    }
}

fn secret_label(kind: &str) -> &'static str {
    match kind { "rancher" => "Token", "kafka" => "Mot de passe", "sonde" => "Jeton de la sonde", _ => "Clé secrète" }
}

/// Brouillon de la fenêtre de connexion : une connexion (schéma de config.yaml) avec un
/// `secret` en plus et un `id` facultatif.
pub fn parse_draft(input: &Value, for_test: bool) -> Result<(Connection, Option<String>), SourceError> {
    let mut map = input.as_object().cloned().ok_or_else(|| SourceError::invalid("Brouillon invalide"))?;
    let secret = map.shift_remove("secret").and_then(|s| s.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty());
    // Pour « Tester », le nom n'est pas encore saisi (il est proposé après le test).
    if for_test && map.get("name").and_then(|n| n.as_str()).is_none_or(str::is_empty) {
        map.insert("name".into(), json!("test"));
    }
    if map.get("id").and_then(|v| v.as_str()).is_none_or(str::is_empty) {
        map.insert("id".into(), json!("nouvelle"));
    }
    match connection_schema().parse_into::<Connection>(&Value::Object(map)) {
        Ok(c) => Ok((c, secret)),
        Err(issues) => {
            let issue = &issues[0];
            let label = issue.path.first().and_then(|s| field_label(&s.to_string())).map(str::to_string).unwrap_or_else(|| issue.path_string());
            Err(SourceError::invalid(if label.is_empty() { issue.message.clone() } else { format!("{label} : {}", issue.message) }))
        }
    }
}

/// Forme écrite dans config.yaml : `caFile` seulement s'il sert ; `Null` = clé retirée.
pub fn normalize(connection: &Connection) -> Map<String, Value> {
    let tls = |t: &crate::config::Tls| {
        let mut v = json!({ "insecure": t.insecure });
        if !t.insecure {
            if let Some(ca) = &t.ca_file { v["caFile"] = json!(ca); }
            if let Some(fp) = &t.fingerprint { v["fingerprint"] = json!(fp); }
        }
        v
    };
    let mut record = serde_json::to_value(connection).unwrap().as_object().cloned().unwrap();
    // Ordre du schéma : id, type, puis les champs.
    let mut out = Map::new();
    out.insert("id".into(), json!(connection.id()));
    out.insert("type".into(), json!(connection.kind()));
    record.shift_remove("id");
    record.shift_remove("type");
    let order: Vec<String> = connection_schema_fields(connection.kind());
    for k in order {
        if let Some(v) = record.shift_remove(&k) { out.insert(k, v); }
    }
    out.extend(record);
    out.insert("tls".into(), tls(connection.tls()));
    match connection {
        Connection::Kafka(k) => {
            out.insert("registry".into(), match &k.registry {
                Some(r) => json!({ "url": r.url, "tls": tls(&r.tls) }),
                None => Value::Null,
            });
        }
        Connection::Rancher(r) => { out.insert("url".into(), json!(r.url.trim_end_matches('/'))); }
        Connection::Sonde(s) => { out.insert("url".into(), json!(s.url.trim_end_matches('/'))); }
        Connection::S3(s) => {
            out.insert("url".into(), json!(s.url.trim_end_matches('/')));
            out.insert("region".into(), s.region.clone().filter(|r| !r.is_empty()).map(Value::String).unwrap_or(Value::Null));
        }
    }
    out
}

fn connection_schema_fields(kind: &str) -> Vec<String> {
    let schema = match kind {
        "rancher" => crate::config::rancher_connection_schema(),
        "kafka" => crate::config::kafka_connection_schema(),
        "sonde" => crate::config::sonde_connection_schema(),
        _ => crate::config::s3_connection_schema(),
    };
    schema.fields().iter().map(|(k, _)| k.clone()).collect()
}

/// Ajouts d'une préconfiguration (réponse à l'activation).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PresetSummary {
    pub connections: Vec<String>,
    pub profiles: usize,
    pub skipped: Vec<String>,
    pub services: Vec<String>,
}

pub struct Connections {
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
}

impl Connections {
    pub fn new(config: Arc<ConfigStore>, secrets: Arc<SecretStore>) -> Self { Connections { config, secrets } }

    pub fn get(&self, id: &str) -> Result<Connection, SourceError> {
        self.config
            .config()
            .connections
            .iter()
            .find(|c| c.id() == id)
            .cloned()
            .ok_or_else(|| SourceError::invalid(format!("Connexion inconnue : {id}")))
    }

    pub fn secret(&self, id: &str) -> Result<String, SourceError> {
        self.secrets.get(id).ok_or_else(|| {
            let kind = self.config.config().connections.iter().find(|c| c.id() == id).map(|c| c.kind().to_string()).unwrap_or_default();
            SourceError::new(ErrorCode::Unauthorized, format!("{} pour cette connexion.", missing_secret(&kind)))
        })
    }

    /// Cible réseau d'une connexion Rancher.
    pub fn target(&self, id: &str) -> Result<RancherTarget, SourceError> {
        match self.get(id)? {
            Connection::Rancher(r) => Ok(RancherTarget { url: r.url, tls: r.tls, token: self.secret(id)? }),
            other => Err(SourceError::invalid(format!("{} n'est pas une connexion Rancher.", other.name()))),
        }
    }

    pub fn has_secret(&self, id: &str) -> bool { self.secrets.has(id) }

    /// Brouillon + secret (saisi, sinon celui enregistré).
    fn resolve(&self, input: &Value) -> Result<(Connection, String), SourceError> {
        let (connection, secret) = parse_draft(input, true)?;
        let secret = match secret { Some(s) => s, None => self.secret(connection.id())? };
        Ok((connection, secret))
    }

    /// Teste une connexion ; renvoie un résumé lisible.
    pub async fn test(&self, input: &Value) -> Result<String, SourceError> {
        let (connection, secret) = self.resolve(input)?;
        let summary = test_connection(&connection, &secret).await?;
        track(AnalyticsEvent::new("Connexion", "Test réussi", Some(connection.kind())));
        Ok(summary)
    }

    pub async fn list_clusters(&self, input: &Value) -> Result<Vec<RancherCluster>, SourceError> {
        let (connection, secret) = self.resolve(input)?;
        match connection {
            Connection::Rancher(r) => crate::rancher::list_clusters(&RancherTarget { url: r.url, tls: r.tls, token: secret }).await,
            _ => Err(SourceError::invalid("Pas un Rancher.")),
        }
    }

    /// Crée ou met à jour ; le secret est chiffré à part. Renvoie l'id.
    pub fn save(&self, input: &Value) -> anyhow::Result<String> {
        let (parsed, secret) = parse_draft(input, false)?;
        let is_new = parsed.id() == "nouvelle";
        let id = if is_new { format!("{}-{}", parsed.kind(), &uuid::Uuid::new_v4().to_string()[..8]) } else { parsed.id().to_string() };
        if secret.is_none() && !self.secrets.has(&id) {
            anyhow::bail!("{} obligatoire.", secret_label(parsed.kind()));
        }
        if let Connection::Rancher(r) = &parsed {
            if r.clusters.is_empty() { anyhow::bail!("Cochez au moins un cluster."); }
        }
        if let Some(s) = &secret { self.secrets.set(&id, s)?; }
        let mut record = normalize(&parsed);
        record.insert("id".into(), json!(id));
        let name = record["name"].as_str().unwrap_or_default().to_string();
        let id2 = id.clone();
        self.config.edit(move |doc, current| {
            match current.connections.iter().position(|c| c.id() == id2) {
                None => {
                    let clean: Map<String, Value> = record.into_iter().filter(|(_, v)| !v.is_null()).collect();
                    doc.push(&ypath!["connections"], &Value::Object(clean))?;
                }
                Some(index) => {
                    // Champ par champ, pour garder les commentaires de l'utilisateur.
                    for (key, value) in record {
                        if key == "id" || key == "type" { continue; }
                        if value.is_null() { doc.delete(&ypath!["connections", index, key])?; } else { doc.set(&ypath!["connections", index, key], &value)?; }
                    }
                }
            }
            Ok(())
        })?;
        log::info!(
            "Connexion {} : {name} ({}, {id}){}",
            if is_new { "créée" } else { "modifiée" },
            parsed.kind(),
            if secret.is_some() { ", secret mis à jour" } else { "" }
        );
        track(AnalyticsEvent::new("Connexion", if is_new { "Créée" } else { "Modifiée" }, Some(parsed.kind())));
        Ok(id)
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        if let Some(index) = self.config.config().connections.iter().position(|c| c.id() == id) {
            self.config.edit(move |doc, _| Ok(doc.delete(&ypath!["connections", index])?))?;
        }
        self.secrets.delete(id)?;
        log::info!("Connexion supprimée : {id}");
        track(AnalyticsEvent::new("Connexion", "Supprimée", None));
        Ok(())
    }

    /// Préconfiguration d'une clé d'activation : ajoute les connexions, profils et services
    /// absents (ceux de l'utilisateur priment), sans secret.
    pub fn add_preset(&self, p: &Preset) -> anyhow::Result<PresetSummary> {
        let current = self.config.config();
        let plan = preset::plan(p, &current.connections, &current.profiles, &current.services);
        if !plan.connections.is_empty() || !plan.profiles.is_empty() || !plan.services.is_empty() {
            let plan2 = plan.clone();
            self.config.edit(move |doc, _| {
                for (id, service) in &plan2.services {
                    let tls = &service["tls"];
                    let mut v = json!({ "url": service["url"] });
                    if tls["insecure"] == json!(true) || tls.get("caFile").is_some() { v["tls"] = tls.clone(); }
                    doc.set(&ypath!["services", id.as_str()], &v)?;
                }
                for c in &plan2.connections {
                    let clean: Map<String, Value> = normalize(c).into_iter().filter(|(_, v)| !v.is_null()).collect();
                    doc.push(&ypath!["connections"], &Value::Object(clean))?;
                }
                for profile in &plan2.profiles {
                    doc.push(&ypath!["profiles"], &serde_json::to_value(profile)?)?;
                }
                Ok(())
            })?;
        }
        let services: Vec<String> = plan.services.keys().cloned().collect();
        log::info!(
            "Préconfiguration : {} connexion(s) et {} profil(s) ajoutés{}{}",
            plan.connections.len(),
            plan.profiles.len(),
            if services.is_empty() { String::new() } else { format!(", services : {}", services.join(", ")) },
            if plan.skipped.is_empty() { String::new() } else { format!(", déjà présentes : {}", plan.skipped.join(", ")) },
        );
        Ok(PresetSummary {
            connections: plan.connections.iter().map(|c| c.name().to_string()).collect(),
            profiles: plan.profiles.len(),
            skipped: plan.skipped,
            services,
        })
    }

    pub fn import_kubeconfig(&self, path: &str) -> anyhow::Result<RancherDraft> {
        crate::rancher::draft_from_kubeconfig(&std::fs::read_to_string(path)?)
    }
}

/// Vérifie une connexion ; renvoie un résumé lisible.
pub async fn test_connection(connection: &Connection, secret: &str) -> Result<String, SourceError> {
    match connection {
        Connection::Rancher(r) => {
            let user = crate::rancher::test_connection(&RancherTarget { url: r.url.clone(), tls: r.tls.clone(), token: secret.into() }).await?;
            Ok(format!("Connecté en tant que {user}"))
        }
        Connection::Kafka(k) => crate::kafka::test(k, secret).await,
        Connection::S3(s) => crate::s3::test(s, secret).await,
        Connection::Sonde(s) => crate::sonde::client::test(s, secret).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::Encryptor;
    use crate::testing::fake_rancher::{self, FAKE_TOKEN};

    pub struct Reverse;
    impl Encryptor for Reverse {
        fn available(&self) -> bool { true }
        fn encrypt(&self, p: &str) -> anyhow::Result<Vec<u8>> { Ok(p.chars().rev().collect::<String>().into_bytes()) }
        fn decrypt(&self, d: &[u8]) -> anyhow::Result<String> { Ok(String::from_utf8(d.to_vec())?.chars().rev().collect()) }
    }

    fn setup(yaml: &str) -> (tempfile::TempDir, Connections) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.yaml");
        std::fs::write(&file, yaml).unwrap();
        let config = ConfigStore::new(&file);
        let secrets = Arc::new(SecretStore::new(dir.path().join("secrets.json"), Box::new(Reverse), None));
        (dir, Connections::new(config, secrets))
    }

    fn draft(url: &str) -> Value {
        json!({ "type": "rancher", "name": "Rancher test", "url": format!("{url}/"), "secret": FAKE_TOKEN, "tls": { "insecure": true }, "clusters": [{ "id": "c-prod1", "name": "prod" }] })
    }

    fn with(mut v: Value, patch: Value) -> Value {
        for (k, x) in patch.as_object().unwrap() { v[k] = x.clone(); }
        v
    }

    #[tokio::test]
    async fn enregistre_modifie_teste_supprime() {
        let rancher = fake_rancher::start(0).await;
        let (_d, c) = setup("# mes réglages\ntheme: dark\n");
        let id = c.save(&draft(&rancher.url)).unwrap();
        let text = std::fs::read_to_string(&c.config.file).unwrap();
        assert!(text.contains("# mes réglages"));
        assert!(!text.contains(FAKE_TOKEN));
        let saved = &c.config.state().config["connections"][0];
        assert_eq!(saved["id"], json!(id));
        assert_eq!(saved["url"], json!(rancher.url));
        assert_eq!(saved["clusters"], json!([{ "id": "c-prod1", "name": "prod" }]));
        assert_eq!(c.secrets.get(&id).as_deref(), Some(FAKE_TOKEN));

        c.save(&with(draft(&rancher.url), json!({ "id": id, "secret": null, "name": "Renommée" }))).unwrap();
        assert_eq!(c.config.config().connections.len(), 1);
        assert_eq!(c.config.config().connections[0].name(), "Renommée");
        assert_eq!(c.test(&with(draft(&rancher.url), json!({ "id": id, "secret": null }))).await.unwrap(), "Connecté en tant que jdupont");
        c.delete(&id).unwrap();
        assert!(c.config.config().connections.is_empty());
        assert!(!c.has_secret(&id));
    }

    #[tokio::test]
    async fn refus_et_erreurs_typees() {
        let rancher = fake_rancher::start(0).await;
        let (_d, c) = setup("");
        assert!(c.save(&with(draft(&rancher.url), json!({ "secret": null }))).unwrap_err().to_string().contains("Token obligatoire"));
        assert!(c.save(&with(draft(&rancher.url), json!({ "clusters": [] }))).unwrap_err().to_string().contains("cluster"));
        assert_eq!(c.test(&with(draft(&rancher.url), json!({ "secret": "bad" }))).await.unwrap_err().code, ErrorCode::Unauthorized);
        assert!(c.test(&with(draft(&rancher.url), json!({ "name": "" }))).await.is_ok());
        assert_eq!(c.test(&with(draft(&rancher.url), json!({ "url": "pas une url" }))).await.unwrap_err().code, ErrorCode::Invalid);
        let e = c.test(&json!({ "type": "kafka", "name": "K", "brokers": [], "tls": { "insecure": false }, "secret": "p" })).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid);
        assert!(e.message.contains("Brokers"), "{}", e.message);
    }

    #[test]
    fn kafka_et_s3_valeurs_par_defaut() {
        let (_d, c) = setup("");
        let kafka = c
            .save(&json!({
                "type": "kafka", "name": "Kafka prod", "brokers": ["kafka1:9092", "kafka2:9092"],
                "security": { "protocol": "SASL_PLAINTEXT", "mechanism": "SCRAM-SHA-512", "username": "app" },
                "tls": { "insecure": false }, "registry": { "url": "http://registry:8081", "tls": { "insecure": false } },
                "secret": "mot-de-passe",
            }))
            .unwrap();
        let s3 = c
            .save(&json!({
                "type": "s3", "name": "S3 intra", "url": "https://s3.intra/", "region": "", "pathStyle": true, "accessKeyId": "AKIA",
                "tls": { "insecure": true, "caFile": "ignoré.cer" }, "buckets": [], "secret": "clé-secrète",
            }))
            .unwrap();
        let text = std::fs::read_to_string(&c.config.file).unwrap();
        for absent in ["mot-de-passe", "clé-secrète", "ignoré.cer", "region"] { assert!(!text.contains(absent), "{absent}\n{text}"); }
        assert_eq!(c.config.config().connections.iter().map(|x| x.id().to_string()).collect::<Vec<_>>(), vec![kafka.clone(), s3]);
        assert!(kafka.starts_with("kafka-"));
        assert_eq!(c.config.state().config["connections"][1]["url"], json!("https://s3.intra"));
        assert_eq!(c.secret(&kafka).unwrap(), "mot-de-passe");
    }

    #[test]
    fn registre_retire() {
        let (_d, c) = setup("");
        let base = json!({ "type": "kafka", "name": "K", "brokers": ["k:9092"], "tls": { "insecure": false }, "secret": "p" });
        let id = c.save(&with(base.clone(), json!({ "registry": { "url": "http://r:8081", "tls": { "insecure": false } } }))).unwrap();
        c.save(&with(base, json!({ "id": id, "secret": null }))).unwrap();
        assert!(c.config.state().config["connections"][0].get("registry").is_none());
    }

    #[test]
    fn preconfiguration() {
        let (_d, c) = setup("# mes réglages\nconnections:\n  - id: kafka-moi\n    type: kafka\n    name: Le mien\n    brokers: [k1:9092]\n");
        let p = preset::validate(Some(&json!({
            "connections": [
                { "type": "kafka", "name": "Kafka prod", "brokers": ["K1:9092"] },
                { "type": "s3", "name": "S3", "url": "https://s3.intra/", "accessKeyId": "AK" },
            ],
            "profiles": [{ "match": "**", "columns": ["traceId"] }],
        })))
        .unwrap();
        assert_eq!(
            c.add_preset(&p).unwrap(),
            PresetSummary { connections: vec!["S3".into()], profiles: 1, skipped: vec!["Kafka prod".into()], services: vec![] }
        );
        assert!(std::fs::read_to_string(&c.config.file).unwrap().contains("# mes réglages"));
        assert_eq!(c.config.config().connections.iter().map(|x| x.name().to_string()).collect::<Vec<_>>(), vec!["Le mien", "S3"]);
        assert_eq!(c.config.state().config["connections"][1]["url"], json!("https://s3.intra"));
        assert_eq!(c.config.config().profiles.len(), 1);
        assert!(!c.secrets.has(c.config.config().connections[1].id()));
    }
}
