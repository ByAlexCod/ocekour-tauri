//! Client HTTP(S) JSON d'un service : TLS de la configuration, jeton, User-Agent Ocekour.

use crate::config::Tls;
use crate::net;
use serde_json::Value;
use std::time::Duration;

/// Erreur d'un service, avec le statut HTTP éventuel ; message destiné à l'agent.
#[derive(Debug, Clone)]
pub struct ServiceError {
    pub message: String,
    pub status: Option<u16>,
}

impl ServiceError {
    pub fn new(message: impl Into<String>) -> Self { ServiceError { message: message.into(), status: None } }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.message) }
}
impl std::error::Error for ServiceError {}

pub type Result<T> = std::result::Result<T, ServiceError>;

/// Paramètres de requête : valeurs `null`, absentes ou vides ignorées.
pub type Query = Vec<(&'static str, Value)>;

/// Message d'erreur lisible dans les réponses des différents serveurs.
pub fn message_of(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(json) => {
            let mut parts: Vec<String> = vec![];
            fn push(v: &Value, parts: &mut Vec<String>) {
                match v {
                    Value::String(s) if !s.is_empty() => parts.push(s.clone()),
                    Value::Array(a) => a.iter().for_each(|x| push(x, parts)),
                    Value::Object(o) => {
                        for (k, x) in o {
                            if let Value::String(s) = x { parts.push(format!("{k} : {s}")); } else { push(x, parts); }
                        }
                    }
                    _ => {}
                }
            }
            for key in ["message", "error", "errorMessages", "errors"] { push(&json[key], &mut parts); }
            parts.join(" ; ")
        }
        Err(_) => {
            static TAG: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"<[^>]+>").unwrap());
            static WS: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"\s+").unwrap());
            let text = TAG.replace_all(body, " ");
            WS.replace_all(&text, " ").trim().chars().take(300).collect()
        }
    }
}

#[derive(Clone)]
pub struct ServiceHttp {
    /// « GitLab », « Jira »… (messages d'erreur).
    pub label: String,
    pub url: String,
    pub tls: Tls,
    pub auth: Vec<(String, String)>,
    pub user_agent: String,
}

/// Corps d'une requête.
pub enum Body {
    None,
    Json(Value),
    Text(String),
}

impl ServiceHttp {
    pub fn base_url(&self) -> String { self.url.trim_end_matches('/').to_string() }

    /// URL web absolue à partir d'un chemin du serveur.
    pub fn web(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") { path.into() } else { format!("{}/{}", self.base_url(), path.trim_start_matches('/')) }
    }

    pub async fn get(&self, path: &str, query: Query) -> Result<Value> {
        self.request("GET", path, query, Body::None, false).await
    }

    pub async fn get_text(&self, path: &str, query: Query) -> Result<String> {
        self.request("GET", path, query, Body::None, true).await.map(|v| v.as_str().unwrap_or_default().to_string())
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request("POST", path, vec![], Body::Json(body), false).await
    }

    pub async fn post_empty(&self, path: &str) -> Result<Value> {
        self.request("POST", path, vec![], Body::None, false).await
    }

    pub async fn put(&self, path: &str, body: Option<Value>, query: Query) -> Result<Value> {
        self.request("PUT", path, query, body.map(Body::Json).unwrap_or(Body::None), false).await
    }

    /// Requête ; `text` = réponse texte (renvoyée en `Value::String`).
    pub async fn request(&self, method: &str, path: &str, query: Query, body: Body, text: bool) -> Result<Value> {
        let label = &self.label;
        let mut url = url::Url::parse(&self.web(path)).map_err(|_| ServiceError::new(format!("{label} : URL invalide ({}).", self.url)))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in &query {
                let s = match v {
                    Value::Null => continue,
                    Value::String(s) if s.is_empty() => continue,
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                pairs.append_pair(k, &s);
            }
        }
        if url.query() == Some("") { url.set_query(None); }
        let client = net::http_client(&self.tls).map_err(|e| ServiceError::new(format!("{label} : {}", e.message)))?;
        let mut req = client
            .request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), url.clone())
            .timeout(Duration::from_secs(30))
            .header("Accept", if text { "*/*" } else { "application/json" })
            .header("User-Agent", &self.user_agent)
            // Jira / Confluence : protection XSRF des requêtes d'écriture.
            .header("X-Atlassian-Token", "no-check");
        for (k, v) in &self.auth { req = req.header(k, v); }
        req = match body {
            Body::None => req,
            Body::Json(v) => req.header("Content-Type", "application/json; charset=utf-8").body(v.to_string()),
            Body::Text(t) => req.header("Content-Type", "text/plain").body(t),
        };
        let res = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ServiceError::new(format!("{label} : pas de réponse après 30 s."))
            } else {
                ServiceError::new(format!("{label} : {}", net::from_reqwest(&e, url.as_str()).message))
            }
        })?;
        let status = res.status().as_u16();
        let location = res.headers().get("location").and_then(|l| l.to_str().ok()).map(str::to_string);
        let body = res.text().await.unwrap_or_default();
        if (300..400).contains(&status) {
            if let Some(l) = location {
                return Err(ServiceError {
                    message: format!("{label} redirige vers {l} : vérifiez l'URL du service (Paramètres → Comptes)."),
                    status: Some(status),
                });
            }
        }
        if !(200..300).contains(&status) { return Err(self.status_error(status, &body, method, url.path())); }
        if text { return Ok(Value::String(body)); }
        if body.is_empty() { return Ok(Value::Null); }
        serde_json::from_str(&body).map_err(|_| ServiceError {
            message: format!("{label} n'a pas répondu en JSON sur {} : vérifiez l'URL du service.", url.path()),
            status: Some(status),
        })
    }

    fn status_error(&self, status: u16, body: &str, method: &str, path: &str) -> ServiceError {
        let label = &self.label;
        let detail = message_of(body);
        let message = match status {
            401 => format!("Jeton {label} refusé (expiré, révoqué ou invalide) : Ocekour → Paramètres → Comptes."),
            403 => format!("{label} : accès refusé (403) sur {method} {path}{}.", if detail.is_empty() { String::new() } else { format!(" — {detail}") }),
            404 => format!("{label} : introuvable (404) : {method} {path}{}.", if detail.is_empty() { String::new() } else { format!(" — {detail}") }),
            _ => format!("{label} a répondu {status} sur {method} {path}{}.", if detail.is_empty() { String::new() } else { format!(" : {detail}") }),
        };
        ServiceError { message, status: Some(status) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_d_erreur() {
        assert_eq!(message_of(r#"{"message":"404 Not Found"}"#), "404 Not Found");
        assert_eq!(message_of(r#"{"errorMessages":["Issue does not exist"],"errors":{"summary":"required"}}"#), "Issue does not exist ; summary : required");
        assert_eq!(message_of("<html><body><h1>Erreur</h1>  serveur</body></html>"), "Erreur serveur");
    }
}
