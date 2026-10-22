//! Serveur MCP (Model Context Protocol) : les mêmes actions que l'interface, pour les agents.
//! Protocole JSON-RPC 2.0, transports stdio (`--mcp`) et HTTP sans session (`--mcp-http`).

pub mod http;
pub mod logs;
pub mod stdio;
pub mod tools;

use crate::analytics::{AnalyticsEvent, track};
use crate::app::App;
use crate::schema::{S, input_schema};
use serde::Serialize;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Réponse d'un outil MCP.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Content {
    pub content: Vec<Value>,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

/// Texte (ou JSON indenté) renvoyé à l'agent.
pub fn ok(value: impl Serialize) -> Content {
    let v = serde_json::to_value(value).unwrap_or(Value::Null);
    let text = match &v { Value::String(s) => s.clone(), other => serde_json::to_string_pretty(other).unwrap_or_default() };
    Content { content: vec![json!({ "type": "text", "text": text })], is_error: false }
}

pub fn fail(message: impl Into<String>) -> Content {
    Content { content: vec![json!({ "type": "text", "text": message.into() })], is_error: true }
}

/// Image (capture d'écran) en base64, avec une légende.
pub fn image(data: String, mime: &str, caption: Option<&str>) -> Content {
    let mut content = vec![];
    if let Some(c) = caption { content.push(json!({ "type": "text", "text": c })); }
    content.push(json!({ "type": "image", "data": data, "mimeType": mime }));
    Content { content, is_error: false }
}

/// Objet JSON sans les valeurs `null` (champs « absents »).
#[macro_export]
macro_rules! obj {
    ($($k:expr => $v:expr),* $(,)?) => {{
        let mut m = serde_json::Map::new();
        $( let v: serde_json::Value = serde_json::to_value(&$v).unwrap_or(serde_json::Value::Null); if !v.is_null() { m.insert($k.to_string(), v); } )*
        serde_json::Value::Object(m)
    }};
}

/// Client MCP à l'origine d'un appel (journal des actions).
#[derive(Debug, Clone, Default)]
pub struct CallInfo {
    pub client: Option<String>,
}

/// Quand un outil est visible.
#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Always,
    /// Service avec jeton (et écriture autorisée si `write`).
    Service(&'static str, bool),
    /// Contexte IA activé.
    Contexte,
    /// Accès web activé.
    Web,
    /// Au moins une base de données configurée.
    Bdd,
    /// Au moins un fichier du poste ouvert aux agents.
    Fichiers,
}

pub type ToolFuture = Pin<Box<dyn Future<Output = anyhow::Result<Content>> + Send>>;
pub type Run = Arc<dyn Fn(Arc<App>, Value, CallInfo) -> ToolFuture + Send + Sync>;

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub title: String,
    pub description: String,
    pub input: Vec<(String, S)>,
    pub read_only: bool,
    pub destructive: bool,
    pub needs_licence: bool,
    pub visibility: Visibility,
    pub run: Run,
}

/// Déclaration d'un outil.
pub fn tool(name: &str, title: &str, description: &str) -> Tool {
    Tool {
        name: name.into(),
        title: title.into(),
        description: description.into(),
        input: vec![],
        read_only: false,
        destructive: false,
        needs_licence: true,
        visibility: Visibility::Always,
        run: Arc::new(|_, _, _| Box::pin(async { Ok(fail("non implémenté")) })),
    }
}

impl Tool {
    pub fn input(mut self, fields: Vec<(&str, S)>) -> Self {
        self.input = fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        self
    }
    pub fn read_only(mut self) -> Self { self.read_only = true; self }
    pub fn destructive(mut self) -> Self { self.destructive = true; self }
    pub fn no_licence(mut self) -> Self { self.needs_licence = false; self }
    pub fn visible(mut self, v: Visibility) -> Self { self.visibility = v; self }
    pub fn run<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Arc<App>, Value, CallInfo) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<Content>> + Send + 'static,
    {
        self.run = Arc::new(move |app, args, call| Box::pin(f(app, args, call)));
        self
    }

    pub fn is_visible(&self, app: &App) -> bool {
        let config = app.config.config();
        match self.visibility {
            Visibility::Always => true,
            Visibility::Service(id, write) => if write { app.accounts.writable(id) } else { app.accounts.available(id) },
            Visibility::Contexte => config.contexte.enabled,
            Visibility::Web => config.web.enabled,
            Visibility::Bdd => !config.databases.is_empty(),
            Visibility::Fichiers => !config.fichiers.agents.is_empty(),
        }
    }

    pub fn describe(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": input_schema(&self.input),
            "annotations": { "readOnlyHint": self.read_only, "destructiveHint": self.destructive, "openWorldHint": true },
        })
    }
}

const INSTRUCTIONS: &str = "Ocekour : visionneuse de logs Rancher, de topics Kafka et de buckets S3. Commencez par \
connexions_lister, puis sources_parcourir pour trouver un conteneur, un topic ou un bucket, et logs_lire ou s3_lister. \
Les secrets ne sont jamais renvoyés.";

fn service_instructions(id: &str) -> &'static str {
    match id {
        "gitlab" => "GitLab (outils gitlab_*) : projets désignés par « groupe/projet » ou URL web. Les commentaires de MR peuvent viser une ligne du diff (gitlab_mr_diff donne les numéros).",
        "jira" => "Jira (outils jira_*) : recherche JQL avec jira_chercher, tickets par clé (PAY-123).",
        "confluence" => "Confluence (outils confluence_*) : recherche CQL ou texte, pages lues en Markdown ; confluence_page_modifier exige la version lue.",
        _ => "Rocket.Chat (outils rocketchat_*) : salons #canal ou @utilisateur ; rocketchat_canaux les liste.",
    }
}

/// Instructions du serveur (selon ce qui est configuré).
pub fn instructions(app: &App) -> String {
    let config = app.config.config();
    let services: Vec<&str> = crate::config::SERVICE_IDS.iter().copied().filter(|id| app.accounts.available(id)).collect();
    let mut parts = vec![INSTRUCTIONS.to_string()];
    if config.contexte.enabled {
        parts.push("Contexte utilisateur (outils contexte_*) : messages Rocket.Chat reçus et nouveautés GitLab / Jira / Confluence. Pour une question sur une MR, un ticket, une page ou une conversation, commencez par contexte_chercher ou contexte_recents.".into());
    }
    if config.web.enabled {
        parts.push("Web / navigateur (outils web_*) : web_recuperer lit une URL sans navigateur (→ Markdown) ; web_naviguer ouvre une page et renvoie un snapshot où chaque élément a une ref [eN] ; ciblez ces refs avec web_cliquer / web_saisir, et web_capture pour une image.".into());
    }
    if !config.databases.is_empty() {
        parts.push("Bases de données (outils bdd_*) : LECTURE SEULE. bdd_connexions liste les bases ; bdd_tables / bdd_colonnes / bdd_apercu pour explorer ; bdd_requete exécute un SELECT (toute écriture est refusée).".into());
    }
    parts.extend(services.iter().map(|id| service_instructions(id).to_string()));
    if !services.is_empty() {
        parts.push("Toute écriture (commentaire, ticket, page, message) est signée « via Ocekour » et inscrite au journal local de l'utilisateur ; les écritures ne sont visibles que si l'utilisateur les a autorisées.".into());
    }
    parts.join("\n\n")
}

const PROTOCOL_VERSIONS: [&str; 5] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05", "2024-10-07"];

/// Session MCP (une par connexion stdio, une par requête HTTP).
#[derive(Clone)]
pub struct Session {
    pub app: Arc<App>,
    pub tools: Arc<Vec<Tool>>,
    /// Client déclaré à l'initialisation (`claude-code 2.1.0`).
    pub client: Option<String>,
    /// En-tête User-Agent (HTTP) : client à défaut.
    pub user_agent: Option<String>,
}

pub(crate) fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

impl Session {
    pub fn new(app: Arc<App>) -> Self {
        let tools = app.mcp_tools();
        Session { app, tools, client: None, user_agent: None }
    }

    pub fn visible_tools(&self) -> Vec<&Tool> { self.tools.iter().filter(|t| t.is_visible(&self.app)).collect() }

    /// Traite un message ; `None` pour une notification (pas de réponse).
    pub async fn handle(&mut self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message["method"].as_str().unwrap_or_default().to_string();
        let params = message.get("params").cloned().unwrap_or(json!({}));
        let Some(id) = id else {
            // Notification (initialized, cancelled…) : rien à répondre.
            return None;
        };
        if message.get("method").is_none() {
            // Réponse du client à une requête du serveur : ignorée.
            return None;
        }
        let result = match method.as_str() {
            "initialize" => {
                let info = &params["clientInfo"];
                if let Some(name) = info["name"].as_str() {
                    self.client = Some(format!("{name} {}", info["version"].as_str().unwrap_or_default()).trim().to_string());
                }
                let requested = params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSIONS[0]);
                let version = if PROTOCOL_VERSIONS.contains(&requested) { requested } else { PROTOCOL_VERSIONS[0] };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": true } },
                    "serverInfo": { "name": "ocekour", "title": "Ocekour", "version": self.app.version },
                    "instructions": instructions(&self.app),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.visible_tools().iter().map(|t| t.describe()).collect::<Vec<_>>() })),
            "tools/call" => self.call(&params).await,
            _ => Err((-32601, "Method not found".to_string())),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => rpc_error(id, code, &message),
        })
    }

    async fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params["name"].as_str().unwrap_or_default();
        let Some(tool) = self.tools.iter().find(|t| t.name == name) else {
            return Err((-32602, format!("MCP error -32602: Tool {name} not found")));
        };
        if !tool.is_visible(&self.app) {
            return Err((-32602, format!("MCP error -32602: Tool {name} disabled")));
        }
        let args = params.get("arguments").cloned().filter(|a| !a.is_null()).unwrap_or(json!({}));
        let content = self.run(tool, args).await;
        track(AnalyticsEvent::new("MCP", &tool.name, Some(if content.is_error { "erreur" } else { "ok" })));
        Ok(serde_json::to_value(content).unwrap())
    }

    async fn run(&self, tool: &Tool, args: Value) -> Content {
        self.app.licence_loaded().await;
        if tool.needs_licence && !self.app.licence.usable() {
            let state = self.app.licence.state();
            return fail(if state.status == "blocked" {
                format!("Ocekour est bloqué : {}", state.message.unwrap_or_default())
            } else {
                "Ocekour n'est pas activé : utilisez l'outil licence_activer avec la clé d'activation.".to_string()
            });
        }
        let schema = crate::schema::object(tool.input.iter().map(|(k, v)| (k.as_str(), v.clone())).collect());
        let args = match schema.parse(&args) {
            Ok(a) => a,
            Err(issues) => {
                return fail(format!(
                    "MCP error -32602: Input validation error: Invalid arguments for tool {} : {}",
                    tool.name,
                    issues.iter().map(|i| i.describe()).collect::<Vec<_>>().join(" ; ")
                ));
            }
        };
        let call = CallInfo { client: self.client.clone().or_else(|| self.user_agent.as_ref().map(|u| u.chars().take(100).collect())) };
        match (tool.run)(self.app.clone(), args, call).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[mcp:{}] {e}", tool.name);
                fail(e.to_string())
            }
        }
    }

    /// Message ou lot de messages (tableau JSON) ; `None` si rien à répondre.
    pub async fn handle_any(&mut self, message: Value) -> Option<Value> {
        match message {
            Value::Array(batch) => {
                let mut out = vec![];
                for m in batch { if let Some(r) = self.handle(m).await { out.push(r); } }
                if out.is_empty() { None } else { Some(Value::Array(out)) }
            }
            m if m.is_object() => self.handle(m).await,
            _ => Some(rpc_error(Value::Null, -32600, "Invalid Request")),
        }
    }

    /// Noms des outils visibles (détection des changements de liste).
    pub fn visible_names(&self) -> Vec<String> { self.visible_tools().iter().map(|t| t.name.clone()).collect() }
}

/// Arguments : accès typés.
pub trait Args {
    fn str(&self, k: &str) -> Option<String>;
    fn string(&self, k: &str) -> String { self.str(k).unwrap_or_default() }
    fn int(&self, k: &str) -> Option<i64>;
    fn bool(&self, k: &str) -> Option<bool>;
}

impl Args for Value {
    fn str(&self, k: &str) -> Option<String> { self.get(k).and_then(|v| v.as_str()).map(str::to_string) }
    fn int(&self, k: &str) -> Option<i64> { self.get(k).and_then(|v| v.as_i64()) }
    fn bool(&self, k: &str) -> Option<bool> { self.get(k).and_then(|v| v.as_bool()) }
}
