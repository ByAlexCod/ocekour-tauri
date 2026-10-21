//! `config.yaml` : schéma (source de vérité), lecture validée avec erreurs positionnées,
//! écriture qui conserve les commentaires, rechargement quand le fichier change ailleurs.

use crate::schema::{self as z, Issue, S, Seg};
use crate::yaml::{self, Editor, P};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// --- Schéma ------------------------------------------------------------------------------------

pub fn tls_schema() -> S {
    z::object(vec![
        ("insecure", z::boolean().default(json!(false)).describe("Ne vérifie pas le certificat du serveur.")),
        ("caFile", z::string().optional().describe("Certificat d'autorité (.cer / .crt / .pem, DER ou PEM).")),
    ])
}

fn tls_default() -> S { tls_schema().default(json!({ "insecure": false })) }

pub fn rancher_connection_schema() -> S {
    z::object(vec![
        ("id", z::string().min(1)),
        ("type", z::literal(json!("rancher"))),
        ("name", z::string().min(1)),
        ("url", z::string().url()),
        ("tls", tls_default()),
        ("clusters", z::array(z::object(vec![("id", z::string().min(1)), ("name", z::string())])).default(json!([]))),
    ])
}

pub fn kafka_connection_schema() -> S {
    z::object(vec![
        ("id", z::string().min(1)),
        ("type", z::literal(json!("kafka"))),
        ("name", z::string().min(1)),
        ("brokers", z::array(z::string().min(1)).min(1)),
        (
            "security",
            z::object(vec![
                ("protocol", z::enumeration(&["SASL_PLAINTEXT", "SASL_SSL", "PLAINTEXT"]).default(json!("SASL_PLAINTEXT"))),
                ("mechanism", z::enumeration(&["SCRAM-SHA-512", "SCRAM-SHA-256", "PLAIN"]).default(json!("SCRAM-SHA-512"))),
                ("username", z::string().optional()),
            ])
            .default(json!({ "protocol": "SASL_PLAINTEXT", "mechanism": "SCRAM-SHA-512" })),
        ),
        ("tls", tls_default()),
        ("registry", z::object(vec![("url", z::string().url()), ("tls", tls_default())]).optional()),
    ])
}

pub fn s3_connection_schema() -> S {
    z::object(vec![
        ("id", z::string().min(1)),
        ("type", z::literal(json!("s3"))),
        ("name", z::string().min(1)),
        ("url", z::string().url()),
        ("region", z::string().optional()),
        ("pathStyle", z::boolean().default(json!(true))),
        ("accessKeyId", z::string().min(1)),
        ("tls", tls_default()),
        ("buckets", z::array(z::string().min(1)).default(json!([]))),
    ])
}

pub fn connection_schema() -> S {
    z::disc("type", vec![
        ("rancher", rancher_connection_schema()),
        ("kafka", kafka_connection_schema()),
        ("s3", s3_connection_schema()),
    ])
}

pub const SERVICE_IDS: [&str; 4] = ["gitlab", "jira", "confluence", "rocketchat"];
pub const NOTIFICATION_SOURCES: [&str; 3] = ["gitlab", "jira", "confluence"];
pub const CONTEXT_SOURCES: [&str; 4] = ["rocketchat", "gitlab", "jira", "confluence"];
pub const DATABASE_ENGINES: [&str; 3] = ["postgres", "sqlserver", "oracle"];

pub fn service_schema() -> S {
    z::object(vec![
        ("url", z::string().url()),
        ("tls", tls_default()),
        ("enabled", z::boolean().default(json!(true))),
        ("write", z::boolean().default(json!(false))),
    ])
}

pub fn services_schema() -> S {
    z::object(SERVICE_IDS.iter().map(|id| (*id, service_schema().optional())).collect())
}

fn notification_source() -> S {
    z::object(vec![
        ("enabled", z::boolean().default(json!(true))),
        ("events", z::array(z::string()).default(json!([]))),
    ])
    .default(json!({}))
}

pub fn notifications_schema() -> S {
    z::object(vec![
        ("enabled", z::boolean().default(json!(false))),
        ("intervalSeconds", z::int().min(30).max(3_600).default(json!(60))),
        ("target", z::string().default(json!("self"))),
        ("desktop", z::boolean().default(json!(true))),
        (
            "sources",
            z::object(NOTIFICATION_SOURCES.iter().map(|s| (*s, notification_source())).collect()).default(json!({})),
        ),
    ])
}

pub fn contexte_schema() -> S {
    z::object(vec![
        ("enabled", z::boolean().default(json!(false))),
        ("intervalSeconds", z::int().min(30).max(3_600).default(json!(60))),
        ("llm", z::enumeration(&["auto", "qwen", "vibe", "off"]).default(json!("auto"))),
        ("tls", tls_default()),
        (
            "sources",
            z::object(
                CONTEXT_SOURCES.iter().map(|s| (*s, z::object(vec![("enabled", z::boolean().default(json!(true)))]).default(json!({})))).collect(),
            )
            .default(json!({})),
        ),
    ])
}

pub fn web_schema() -> S {
    z::object(vec![
        ("enabled", z::boolean().default(json!(false))),
        ("browser", z::enumeration(&["msedge", "chrome", "chromium"]).default(json!("msedge"))),
        ("headless", z::boolean().default(json!(false))),
        ("timeoutMs", z::int().min(1_000).max(120_000).default(json!(30_000))),
        ("tls", tls_default()),
    ])
}

pub fn database_schema() -> S {
    z::object(vec![
        ("id", z::string().min(1)),
        ("engine", z::enumeration(&DATABASE_ENGINES)),
        ("name", z::string().min(1)),
        ("host", z::string().min(1)),
        ("port", z::int().min(1).max(65_535).optional()),
        ("database", z::string().optional()),
        ("user", z::string().min(1)),
        ("ssl", z::boolean().default(json!(false))),
        ("oracleSid", z::boolean().default(json!(false))),
        ("tls", tls_default()),
    ])
}

pub fn profile_schema() -> S {
    z::object(vec![
        ("match", z::string().min(1)),
        (
            "fields",
            z::object(vec![
                ("time", z::string().optional()),
                ("level", z::string().optional()),
                ("message", z::string().optional()),
            ])
            .default(json!({})),
        ),
        ("columns", z::array(z::string()).default(json!([]))),
        ("expand", z::array(z::string()).default(json!([]))),
        ("wrap", z::array(z::string()).default(json!([]))),
        ("parseNested", z::boolean().optional()),
    ])
}

pub fn stored_filter_rule_schema() -> S {
    z::object(vec![
        ("enabled", z::boolean().default(json!(true))),
        ("field", z::string().min(1).default(json!("path"))),
        ("mode", z::enumeration(&["exact", "contains", "regex"]).default(json!("contains"))),
        ("value", z::string().default(json!(""))),
        ("caseSensitive", z::boolean().default(json!(false))),
        ("action", z::enumeration(&["hide", "only"]).default(json!("hide"))),
    ])
}

pub fn preferences_schema() -> S {
    z::object(vec![
        ("theme", z::enumeration(&["system", "dark", "light"]).default(json!("system"))),
        ("bufferSize", z::int().min(1_000).max(1_000_000).default(json!(100_000))),
        ("defaultTailLines", z::int().min(0).max(1_000_000).default(json!(1_000))),
        ("restoreTabs", z::boolean().default(json!(true))),
        ("updateShare", z::string().optional()),
        ("mcpServer", z::boolean().default(json!(false))),
        ("mcpPort", z::int().min(1_024).max(65_535).default(json!(47_110))),
    ])
}

pub fn config_schema() -> S {
    preferences_schema().extend(vec![
        ("connections", z::array(connection_schema()).default(json!([]))),
        ("profiles", z::array(profile_schema()).default(json!([]))),
        ("favorites", z::array(z::string()).default(json!([]))),
        ("services", services_schema().default(json!({}))),
        ("notifications", notifications_schema().default(json!({}))),
        ("contexte", contexte_schema().default(json!({}))),
        (
            "filterPresets",
            z::array(z::object(vec![
                ("name", z::string().min(1)),
                ("rules", z::array(stored_filter_rule_schema()).default(json!([]))),
            ]))
            .default(json!([])),
        ),
        ("web", web_schema().default(json!({}))),
        ("databases", z::array(database_schema()).default(json!([]))),
    ])
}

/// Clés de préférences (`config:setPreferences`).
pub const PREFERENCE_KEYS: [&str; 7] =
    ["theme", "bufferSize", "defaultTailLines", "restoreTabs", "updateShare", "mcpServer", "mcpPort"];

// --- Types ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tls {
    #[serde(default)]
    pub insecure: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClusterRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RancherConnection {
    pub id: String,
    pub name: String,
    pub url: String,
    pub tls: Tls,
    pub clusters: Vec<ClusterRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KafkaSecurity {
    pub protocol: String,
    pub mechanism: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub url: String,
    pub tls: Tls,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KafkaConnection {
    pub id: String,
    pub name: String,
    pub brokers: Vec<String>,
    pub security: KafkaSecurity,
    pub tls: Tls,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<Registry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct S3Connection {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub path_style: bool,
    pub access_key_id: String,
    pub tls: Tls,
    pub buckets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Connection {
    Rancher(RancherConnection),
    Kafka(KafkaConnection),
    S3(S3Connection),
}

impl Connection {
    pub fn id(&self) -> &str {
        match self { Connection::Rancher(c) => &c.id, Connection::Kafka(c) => &c.id, Connection::S3(c) => &c.id }
    }
    pub fn name(&self) -> &str {
        match self { Connection::Rancher(c) => &c.name, Connection::Kafka(c) => &c.name, Connection::S3(c) => &c.name }
    }
    pub fn kind(&self) -> &'static str {
        match self { Connection::Rancher(_) => "rancher", Connection::Kafka(_) => "kafka", Connection::S3(_) => "s3" }
    }
    pub fn tls(&self) -> &Tls {
        match self { Connection::Rancher(c) => &c.tls, Connection::Kafka(c) => &c.tls, Connection::S3(c) => &c.tls }
    }
    /// URL (Rancher, S3) ou premier broker (Kafka) : messages d'erreur.
    pub fn address(&self) -> String {
        match self {
            Connection::Rancher(c) => c.url.clone(),
            Connection::Kafka(c) => c.brokers.first().cloned().unwrap_or_default(),
            Connection::S3(c) => c.url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceConfig {
    pub url: String,
    pub tls: Tls,
    pub enabled: bool,
    pub write: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Services {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitlab: Option<ServiceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jira: Option<ServiceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confluence: Option<ServiceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rocketchat: Option<ServiceConfig>,
}

impl Services {
    pub fn get(&self, id: &str) -> Option<&ServiceConfig> {
        match id {
            "gitlab" => self.gitlab.as_ref(),
            "jira" => self.jira.as_ref(),
            "confluence" => self.confluence.as_ref(),
            "rocketchat" => self.rocketchat.as_ref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceToggle {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub target: String,
    pub desktop: bool,
    pub sources: std::collections::BTreeMap<String, SourceToggle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContexteConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub llm: String,
    pub tls: Tls,
    pub sources: std::collections::BTreeMap<String, SourceToggle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebConfig {
    pub enabled: bool,
    pub browser: String,
    pub headless: bool,
    pub timeout_ms: u64,
    pub tls: Tls,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConnection {
    pub id: String,
    pub engine: String,
    pub name: String,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub user: String,
    pub ssl: bool,
    pub oracle_sid: bool,
    pub tls: Tls,
}

impl DatabaseConnection {
    pub fn port_or_default(&self) -> u16 {
        self.port.unwrap_or(match self.engine.as_str() { "postgres" => 5432, "sqlserver" => 1433, _ => 1521 })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Fields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(rename = "match")]
    pub match_: String,
    #[serde(default)]
    pub fields: Fields,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub expand: Vec<String>,
    #[serde(default)]
    pub wrap: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_nested: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredFilterRule {
    pub enabled: bool,
    pub field: String,
    pub mode: String,
    pub value: String,
    pub case_sensitive: bool,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilterPreset {
    pub name: String,
    pub rules: Vec<StoredFilterRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub theme: String,
    pub buffer_size: u64,
    pub default_tail_lines: u64,
    pub restore_tabs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_share: Option<String>,
    pub mcp_server: bool,
    pub mcp_port: u16,
    pub connections: Vec<Connection>,
    pub profiles: Vec<Profile>,
    pub favorites: Vec<String>,
    pub services: Services,
    pub notifications: NotificationsConfig,
    pub contexte: ContexteConfig,
    pub filter_presets: Vec<FilterPreset>,
    pub web: WebConfig,
    pub databases: Vec<DatabaseConnection>,
}

impl Default for AppConfig {
    fn default() -> Self {
        config_schema().parse_into(&json!({})).expect("configuration par défaut")
    }
}

// --- Lecture ----------------------------------------------------------------------------------

/// Erreur de config.yaml, positionnée si possible (lignes et colonnes à partir de 1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<usize>,
}

/// Configuration validée : forme JSON (pour l'interface) et forme typée.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub json: Value,
    pub config: AppConfig,
}

fn issue_path(issue: &Issue) -> Vec<P> {
    issue.path.iter().map(|s| match s { Seg::Key(k) => P::K(k.clone()), Seg::Index(i) => P::I(*i) }).collect()
}

/// Parse + valide un texte YAML ; les erreurs sont positionnées dans le texte.
pub fn parse_config(text: &str) -> Result<Parsed, Vec<ConfigError>> {
    let doc = yaml::parse(text).map_err(|e| vec![ConfigError { message: e.message, line: Some(e.line), col: Some(e.col) }])?;
    let mut raw = doc.to_json();
    if raw.is_null() { raw = json!({}); }
    let schema = config_schema();
    match schema.parse(&raw) {
        Ok(json) => {
            let config: AppConfig = serde_json::from_value(json.clone())
                .map_err(|e| vec![ConfigError { message: e.to_string(), line: None, col: None }])?;
            Ok(Parsed { json, config })
        }
        Err(issues) => Err(issues
            .iter()
            .map(|i| {
                let pos = doc.position(&issue_path(i));
                ConfigError { message: i.describe(), line: pos.map(|p| p.0), col: pos.map(|p| p.1) }
            })
            .collect()),
    }
}

// --- Magasin ----------------------------------------------------------------------------------

/// État publié à l'interface.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigState {
    /// Dernière configuration valide (celle qui est appliquée).
    pub config: Value,
    /// Erreurs du fichier sur disque ; vide s'il est valide.
    pub errors: Vec<ConfigError>,
    pub path: String,
}

/// Résultat d'un enregistrement de texte complet.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SaveResult {
    Ok { ok: bool, state: ConfigState },
    Err { ok: bool, errors: Vec<ConfigError> },
}

type Listener = Box<dyn Fn(&ConfigState) + Send + Sync>;

struct Inner {
    json: Value,
    config: Arc<AppConfig>,
    errors: Vec<ConfigError>,
    loaded_text: String,
}

/// config.yaml : dernière configuration valide en mémoire, écriture qui conserve les
/// commentaires, rechargement à chaud.
pub struct ConfigStore {
    pub file: PathBuf,
    inner: RwLock<Inner>,
    listeners: RwLock<Vec<Listener>>,
    write_lock: parking_lot::Mutex<()>,
}

impl ConfigStore {
    pub fn new(file: impl Into<PathBuf>) -> Arc<Self> {
        let defaults = config_schema().parse(&json!({})).unwrap();
        let store = Arc::new(ConfigStore {
            file: file.into(),
            inner: RwLock::new(Inner {
                config: Arc::new(serde_json::from_value(defaults.clone()).unwrap()),
                json: defaults,
                errors: vec![],
                loaded_text: String::new(),
            }),
            listeners: RwLock::new(vec![]),
            write_lock: parking_lot::Mutex::new(()),
        });
        store.reload();
        store
    }

    /// Configuration typée courante.
    pub fn config(&self) -> Arc<AppConfig> { self.inner.read().config.clone() }

    pub fn state(&self) -> ConfigState {
        let inner = self.inner.read();
        ConfigState { config: inner.json.clone(), errors: inner.errors.clone(), path: self.file.to_string_lossy().into() }
    }

    pub fn has_errors(&self) -> bool { !self.inner.read().errors.is_empty() }

    pub fn on_changed(&self, listener: impl Fn(&ConfigState) + Send + Sync + 'static) {
        self.listeners.write().push(Box::new(listener));
    }

    pub fn read_text(&self) -> std::io::Result<String> {
        match std::fs::read_to_string(&self.file) {
            Ok(t) => Ok(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    pub fn reload(&self) -> ConfigState {
        let text = self.read_text().unwrap_or_default();
        {
            let mut inner = self.inner.write();
            match parse_config(&text) {
                Ok(parsed) => {
                    inner.json = parsed.json;
                    inner.config = Arc::new(parsed.config);
                    inner.errors = vec![];
                }
                Err(errors) => inner.errors = errors,
            }
            inner.loaded_text = text;
        }
        let state = self.state();
        for l in self.listeners.read().iter() { l(&state); }
        state
    }

    /// Remplace tout le fichier (éditeur YAML). Refusé si le texte est invalide.
    pub fn save_text(&self, text: &str) -> SaveResult {
        if let Err(errors) = parse_config(text) {
            return SaveResult::Err { ok: false, errors };
        }
        if let Err(e) = self.write(text) {
            return SaveResult::Err { ok: false, errors: vec![ConfigError { message: e.to_string(), line: None, col: None }] };
        }
        SaveResult::Ok { ok: true, state: self.reload() }
    }

    /// Modifie le document YAML en place (commentaires conservés). Refusé si le fichier sur
    /// disque est invalide, pour ne jamais écraser le travail de l'utilisateur.
    pub fn edit(&self, mutate: impl FnOnce(&mut Editor, &AppConfig) -> anyhow::Result<()>) -> anyhow::Result<ConfigState> {
        let _guard = self.write_lock.lock();
        let text = self.read_text()?;
        if yaml::parse(&text).is_err() || self.has_errors() {
            anyhow::bail!("config.yaml contient des erreurs : corrigez-les avant toute modification.");
        }
        let mut editor = Editor::new(&text);
        let current = self.config();
        mutate(&mut editor, &current)?;
        match self.save_text(&editor.text) {
            SaveResult::Ok { state, .. } => Ok(state),
            SaveResult::Err { errors, .. } => anyhow::bail!("{}", errors[0].message),
        }
    }

    fn write(&self, text: &str) -> std::io::Result<()> {
        if let Some(dir) = self.file.parent() { std::fs::create_dir_all(dir)?; }
        std::fs::write(&self.file, text)
    }

    /// Recharge à chaud sur modification externe. Le surveillant vit tant que la valeur renvoyée
    /// est gardée.
    pub fn watch(self: &Arc<Self>) -> anyhow::Result<notify::RecommendedWatcher> {
        use notify::{RecursiveMode, Watcher};
        let dir = self.file.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&dir)?;
        let name = self.file.file_name().map(|n| n.to_owned());
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if event.paths.iter().any(|p| p.file_name() == name.as_deref()) { let _ = tx.send(()); }
            }
        })?;
        watcher.watch(&dir, RecursiveMode::NonRecursive)?;
        let weak = Arc::downgrade(self);
        std::thread::Builder::new().name("config-watch".into()).spawn(move || {
            while rx.recv().is_ok() {
                // Regroupe les événements rapprochés (éditeurs qui remplacent le fichier).
                std::thread::sleep(std::time::Duration::from_millis(200));
                while rx.try_recv().is_ok() {}
                let Some(store) = weak.upgrade() else { break };
                let text = store.read_text().unwrap_or_default();
                if text != store.inner.read().loaded_text { store.reload(); }
            }
        })?;
        Ok(watcher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ypath;

    #[test]
    fn fichier_vide_valeurs_par_defaut() {
        let p = parse_config("").unwrap();
        assert_eq!(p.config.theme, "system");
        assert_eq!(p.config.buffer_size, 100_000);
        assert_eq!(p.config.default_tail_lines, 1_000);
        assert!(p.config.restore_tabs);
        assert!(p.config.connections.is_empty());
        assert_eq!(p.json["notifications"]["sources"]["gitlab"], json!({ "enabled": true, "events": [] }));
        assert_eq!(p.json["web"]["timeoutMs"], json!(30_000));
        assert_eq!(p.json["contexte"]["tls"], json!({ "insecure": false }));
    }

    #[test]
    fn erreur_de_syntaxe_positionnee() {
        let e = parse_config("theme: dark\nprofiles: [\n").unwrap_err();
        assert!(e[0].line.unwrap() > 0);
    }

    #[test]
    fn erreur_de_schema_sur_la_bonne_ligne() {
        let e = parse_config("theme: dark\nconnections:\n  - id: a\n    type: rancher\n    name: A\n    url: pas-une-url\n").unwrap_err();
        assert!(e[0].message.contains("connections.0.url"), "{}", e[0].message);
        assert_eq!(e[0].line, Some(6));
    }

    #[test]
    fn cle_manquante_position_du_parent() {
        let e = parse_config("profiles:\n  - columns: [a]\n").unwrap_err();
        assert_eq!(e[0].line, Some(2));
    }

    #[test]
    fn connexions_typees() {
        let p = parse_config(
            "connections:\n  - { id: k, type: kafka, name: K, brokers: [a:9092] }\n  - { id: s, type: s3, name: S, url: 'https://s3.intra', accessKeyId: AK }\n",
        )
        .unwrap();
        match &p.config.connections[0] {
            Connection::Kafka(k) => {
                assert_eq!(k.security.protocol, "SASL_PLAINTEXT");
                assert_eq!(k.security.mechanism, "SCRAM-SHA-512");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(p.json["connections"][1]["pathStyle"], json!(true));
        assert!(parse_config("theme: violet\n").is_err());
    }

    fn store(text: Option<&str>) -> (tempfile::TempDir, Arc<ConfigStore>) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.yaml");
        if let Some(t) = text { std::fs::write(&file, t).unwrap(); }
        let s = ConfigStore::new(&file);
        (dir, s)
    }

    #[test]
    fn edit_conserve_les_commentaires() {
        let (_d, s) = store(Some("# mes réglages\ntheme: light # clair le jour\n"));
        s.edit(|doc, _| Ok(doc.set(&ypath!["theme"], &json!("dark"))?)).unwrap();
        let text = std::fs::read_to_string(&s.file).unwrap();
        assert!(text.contains("# mes réglages"));
        assert!(text.contains("theme: dark # clair le jour"));
        assert_eq!(s.config().theme, "dark");
    }

    #[test]
    fn fichier_invalide_garde_la_derniere_config_et_refuse_edit() {
        let (_d, s) = store(Some("theme: dark\n"));
        std::fs::write(&s.file, "theme: violet\n").unwrap();
        s.reload();
        assert_eq!(s.config().theme, "dark");
        assert_eq!(s.state().errors.len(), 1);
        let err = s.edit(|doc, _| Ok(doc.set(&ypath!["bufferSize"], &json!(5000))?)).unwrap_err();
        assert!(err.to_string().contains("erreurs"));
        assert_eq!(std::fs::read_to_string(&s.file).unwrap(), "theme: violet\n");
    }

    #[test]
    fn save_text_refuse_un_texte_invalide() {
        let (_d, s) = store(None);
        assert!(matches!(s.save_text("bufferSize: 3\n"), SaveResult::Err { .. }));
        assert!(!s.file.exists());
    }

    #[test]
    fn notifie_les_modifications_externes() {
        let (_d, s) = store(None);
        let _w = s.watch().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        s.on_changed(move |st| { let _ = tx.send(st.config["theme"].clone()); });
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::write(&s.file, "theme: dark\n").unwrap();
        let theme = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(theme, json!("dark"));
    }
}
