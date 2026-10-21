//! Bases de données (PostgreSQL, SQL Server, Oracle) en LECTURE SEULE : config.yaml pour le
//! paramétrage, coffre à secrets pour les mots de passe. Toute requête passe par la garde.

pub mod drivers;
pub mod guard;
pub mod tools;

use crate::config::{ConfigStore, DatabaseConnection, database_schema};
use crate::error::{ErrorCode, SourceError};
use crate::secrets::SecretStore;
use crate::ypath;
use drivers::{ColumnInfo, QueryResult, TableInfo};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::time::Duration;

const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Vue publique (sans mot de passe) d'une base configurée.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    pub id: String,
    pub engine: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub user: String,
    pub ssl: bool,
    pub has_secret: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DatabaseTestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct DatabaseManager {
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
}

/// Brouillon de la fenêtre « Base de données » : la connexion + un mot de passe facultatif.
fn parse_draft(input: &Value, for_test: bool) -> anyhow::Result<(DatabaseConnection, Option<String>)> {
    let mut map = input.as_object().cloned().ok_or_else(|| anyhow::anyhow!("Objet attendu"))?;
    let password = map.remove("password").and_then(|p| p.as_str().map(str::to_string)).filter(|p| !p.is_empty());
    if for_test && map.get("name").is_none_or(|n| n.as_str().is_none_or(str::is_empty)) { map.insert("name".into(), json!("test")); }
    if map.get("id").is_none_or(|n| n.as_str().is_none_or(str::is_empty)) { map.insert("id".into(), json!("nouvelle")); }
    let conn = database_schema().parse_into(&Value::Object(map)).map_err(|e| anyhow::anyhow!(e[0].describe()))?;
    Ok((conn, password))
}

/// Forme écrite dans config.yaml (caFile seulement s'il sert ; pas de valeurs par défaut inutiles).
fn normalize(c: &DatabaseConnection) -> Map<String, Value> {
    let tls = if c.tls.ca_file.is_some() && !c.tls.insecure { json!(c.tls) } else { json!({ "insecure": c.tls.insecure }) };
    let mut m = Map::new();
    m.insert("id".into(), json!(c.id));
    m.insert("engine".into(), json!(c.engine));
    m.insert("name".into(), json!(c.name));
    m.insert("host".into(), json!(c.host));
    m.insert("port".into(), json!(c.port));
    m.insert("database".into(), json!(c.database.as_ref().filter(|d| !d.is_empty())));
    m.insert("user".into(), json!(c.user));
    m.insert("ssl".into(), if c.ssl { json!(true) } else { Value::Null });
    m.insert("oracleSid".into(), if c.oracle_sid { json!(true) } else { Value::Null });
    m.insert("tls".into(), if c.ssl { tls } else { Value::Null });
    m
}

impl DatabaseManager {
    pub fn new(config: Arc<ConfigStore>, secrets: Arc<SecretStore>) -> Self { DatabaseManager { config, secrets } }

    pub fn list(&self) -> Vec<DatabaseInfo> {
        self.config.config().databases.iter().map(|c| DatabaseInfo {
            id: c.id.clone(), engine: c.engine.clone(), name: c.name.clone(), host: c.host.clone(), port: c.port_or_default(),
            database: c.database.clone(), user: c.user.clone(), ssl: c.ssl, has_secret: self.secrets.has(&c.id),
        }).collect()
    }

    pub fn get(&self, id: &str) -> Result<DatabaseConnection, SourceError> {
        self.config.config().databases.iter().find(|c| c.id == id).cloned().ok_or_else(|| SourceError::invalid(format!("Base de données inconnue : {id}")))
    }

    fn password(&self, id: &str) -> Result<String, SourceError> {
        self.secrets.get(id).ok_or_else(|| SourceError::new(ErrorCode::Unauthorized, "Aucun mot de passe enregistré pour cette base."))
    }

    pub async fn test(&self, input: &Value) -> DatabaseTestResult {
        let result = async {
            let (conn, password) = parse_draft(input, true)?;
            let pw = match password { Some(p) => p, None => self.password(&conn.id)? };
            drivers::test(&conn, &pw).await
        }
        .await;
        match result {
            Ok(summary) => DatabaseTestResult { ok: true, summary: Some(summary), error: None },
            Err(e) => DatabaseTestResult { ok: false, summary: None, error: Some(e.to_string()) },
        }
    }

    pub fn save(&self, input: &Value) -> anyhow::Result<String> {
        let (conn, password) = parse_draft(input, false)?;
        let is_new = conn.id == "nouvelle";
        let id = if is_new { format!("db-{}", &uuid::Uuid::new_v4().to_string()[..8]) } else { conn.id.clone() };
        if password.is_none() && !self.secrets.has(&id) { anyhow::bail!("Mot de passe obligatoire."); }
        if let Some(p) = &password { self.secrets.set(&id, p)?; }
        let mut record = normalize(&conn);
        record.insert("id".into(), json!(id));
        let id2 = id.clone();
        self.config.edit(move |doc, current| {
            match current.databases.iter().position(|c| c.id == id2) {
                None => {
                    let clean: Map<String, Value> = record.into_iter().filter(|(_, v)| !v.is_null()).collect();
                    doc.push(&ypath!["databases"], &Value::Object(clean))?;
                }
                Some(index) => {
                    for (key, value) in record {
                        if key == "id" || key == "engine" { continue; }
                        if value.is_null() { doc.delete(&ypath!["databases", index, key])?; } else { doc.set(&ypath!["databases", index, key], &value)?; }
                    }
                }
            }
            Ok(())
        })?;
        log::info!("Base {} : {} ({}, {id})", if is_new { "créée" } else { "modifiée" }, conn.name, conn.engine);
        Ok(id)
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        if let Some(index) = self.config.config().databases.iter().position(|c| c.id == id) {
            self.config.edit(move |doc, _| Ok(doc.delete(&ypath!["databases", index])?))?;
        }
        self.secrets.delete(id)?;
        log::info!("Base supprimée : {id}");
        Ok(())
    }

    pub fn has_secret(&self, id: &str) -> bool { self.secrets.has(id) }

    pub async fn query(&self, id: &str, sql: &str, max_rows: usize) -> anyhow::Result<QueryResult> {
        let conn = self.get(id)?;
        let clean = guard::assert_read_only(sql)?;
        drivers::query(&conn, &self.password(id)?, &clean, max_rows.clamp(1, 10_000), QUERY_TIMEOUT).await
    }

    pub async fn tables(&self, id: &str) -> anyhow::Result<Vec<TableInfo>> {
        let conn = self.get(id)?;
        drivers::tables(&conn, &self.password(id)?).await
    }

    pub async fn columns(&self, id: &str, table: &str, schema: Option<&str>) -> anyhow::Result<Vec<ColumnInfo>> {
        let conn = self.get(id)?;
        drivers::columns(&conn, &self.password(id)?, table, schema).await
    }
}
