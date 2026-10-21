//! Services accessibles aux agents (GitLab, Jira, Confluence, Rocket.Chat) : comptes (URL dans
//! config.yaml, jeton chiffré), client HTTP, conversions, journal des écritures, outils MCP.

pub mod confluence;
pub mod gitlab;
pub mod http;
pub mod jira;
pub mod journal;
pub mod markup;
pub mod rocketchat;
pub mod tools;

use crate::config::{ConfigStore, SERVICE_IDS, ServiceConfig, Tls};
use crate::secrets::SecretStore;
use crate::ypath;
use http::{ServiceError, ServiceHttp};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// Définition d'un service.
pub struct Definition {
    pub label: &'static str,
    /// Jeton en deux parties : identifiant utilisateur + jeton (Rocket.Chat).
    pub needs_user_id: bool,
    pub token_path: &'static str,
    pub fallback_token_path: Option<&'static str>,
}

pub fn definition(id: &str) -> &'static Definition {
    match id {
        "gitlab" => &Definition {
            label: "GitLab",
            needs_user_id: false,
            token_path: "/-/user_settings/personal_access_tokens?name=Ocekour&scopes=api",
            fallback_token_path: Some("/-/profile/personal_access_tokens?name=Ocekour&scopes=api"),
        },
        "jira" => &Definition {
            label: "Jira",
            needs_user_id: false,
            token_path: "/secure/ViewProfile.jspa?selectedTab=com.atlassian.pats.pats-plugin:jira-user-personal-access-tokens",
            fallback_token_path: None,
        },
        "confluence" => &Definition { label: "Confluence", needs_user_id: false, token_path: "/plugins/personalaccesstokens/usertokens.action", fallback_token_path: None },
        _ => &Definition { label: "Rocket.Chat", needs_user_id: true, token_path: "/account/tokens", fallback_token_path: None },
    }
}

fn secret_id(id: &str) -> String { format!("service:{id}") }

/// Compte tel que vu par l'écran Comptes : jamais le jeton.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAccount {
    pub id: &'static str,
    pub label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub tls: Tls,
    pub enabled: bool,
    pub write: bool,
    pub has_token: bool,
    pub needs_user_id: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct TestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

type Listener = Arc<dyn Fn() + Send + Sync>;

/// Comptes des services : URL et réglages dans config.yaml (`services`), jeton chiffré.
pub struct Accounts {
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    pub user_agent: String,
    listeners: Mutex<Vec<Listener>>,
}

impl Accounts {
    pub fn new(config: Arc<ConfigStore>, secrets: Arc<SecretStore>, user_agent: &str) -> Arc<Self> {
        let accounts = Arc::new(Accounts { config: config.clone(), secrets, user_agent: user_agent.into(), listeners: Mutex::new(vec![]) });
        let weak = Arc::downgrade(&accounts);
        config.on_changed(move |_| {
            if let Some(a) = weak.upgrade() { a.emit(); }
        });
        accounts
    }

    /// Changement de configuration ou de jeton (liste des outils MCP à revoir).
    pub fn on_changed(&self, listener: impl Fn() + Send + Sync + 'static) { self.listeners.lock().push(Arc::new(listener)); }

    fn emit(&self) {
        let listeners: Vec<Listener> = self.listeners.lock().clone();
        for l in listeners { l(); }
    }

    pub fn settings(&self, id: &str) -> Option<ServiceConfig> { self.config.config().services.get(id).cloned() }

    /// Un jeton est enregistré ET déchiffrable.
    pub fn has_token(&self, id: &str) -> bool { self.secrets.readable(&secret_id(id)) }

    /// Outils du service visibles : URL, jeton, service activé.
    pub fn available(&self, id: &str) -> bool { self.settings(id).is_some_and(|s| s.enabled) && self.has_token(id) }

    pub fn writable(&self, id: &str) -> bool { self.available(id) && self.settings(id).is_some_and(|s| s.write) }

    pub fn list(&self) -> Vec<ServiceAccount> {
        SERVICE_IDS
            .iter()
            .map(|id| {
                let s = self.settings(id);
                let def = definition(id);
                ServiceAccount {
                    id,
                    label: def.label,
                    url: s.as_ref().map(|s| s.url.clone()),
                    tls: s.as_ref().map(|s| s.tls.clone()).unwrap_or_default(),
                    enabled: s.as_ref().map(|s| s.enabled).unwrap_or(true),
                    write: s.as_ref().map(|s| s.write).unwrap_or(false),
                    has_token: self.has_token(id),
                    needs_user_id: def.needs_user_id,
                }
            })
            .collect()
    }

    /// Enregistre (ou retire avec une URL vide) l'URL et le TLS d'un service.
    pub fn save(&self, id: &str, url: &str, tls: &Tls) -> anyhow::Result<Vec<ServiceAccount>> {
        let url = url.trim().trim_end_matches('/').to_string();
        let (id2, tls) = (id.to_string(), tls.clone());
        self.config.edit(move |doc, _| {
            if url.is_empty() {
                doc.delete(&ypath!["services", id2.as_str()])?;
                return Ok(());
            }
            doc.set(&ypath!["services", id2.as_str(), "url"], &json!(url))?;
            if tls.insecure || tls.ca_file.is_some() {
                doc.set(&ypath!["services", id2.as_str(), "tls"], &serde_json::to_value(&tls)?)?;
            } else {
                doc.delete(&ypath!["services", id2.as_str(), "tls"])?;
            }
            Ok(())
        })?;
        Ok(self.list())
    }

    pub fn set_flags(&self, id: &str, enabled: Option<bool>, write: Option<bool>) -> anyhow::Result<Vec<ServiceAccount>> {
        if self.settings(id).is_none() { anyhow::bail!("{} : renseignez d'abord l'URL.", definition(id).label); }
        let id2 = id.to_string();
        self.config.edit(move |doc, _| {
            if let Some(e) = enabled { doc.set(&ypath!["services", id2.as_str(), "enabled"], &json!(e))?; }
            if let Some(w) = write { doc.set(&ypath!["services", id2.as_str(), "write"], &json!(w))?; }
            Ok(())
        })?;
        Ok(self.list())
    }

    /// Enregistre le jeton (chiffré) puis le teste.
    pub async fn set_token(&self, id: &str, token: &str, user_id: Option<&str>) -> anyhow::Result<TestResult> {
        let value = token.trim();
        if value.is_empty() { anyhow::bail!("Jeton vide."); }
        let def = definition(id);
        let user = user_id.map(str::trim).unwrap_or_default();
        if def.needs_user_id && user.is_empty() { anyhow::bail!("Rocket.Chat : l'identifiant utilisateur est aussi nécessaire."); }
        let stored = if def.needs_user_id { json!({ "token": value, "userId": user }).to_string() } else { value.to_string() };
        self.secrets.set(&secret_id(id), &stored)?;
        log::info!("Comptes : jeton {} enregistré", def.label);
        self.emit();
        Ok(self.test(id).await)
    }

    pub fn forget_token(&self, id: &str) -> anyhow::Result<Vec<ServiceAccount>> {
        self.secrets.delete(&secret_id(id))?;
        log::info!("Comptes : jeton {} oublié", definition(id).label);
        self.emit();
        Ok(self.list())
    }

    pub async fn test(&self, id: &str) -> TestResult {
        let result = async {
            let http = self.client(id)?;
            let write = self.settings(id).is_some_and(|s| s.write);
            test_service(id, &http, write).await
        }
        .await;
        match result {
            Ok((identity, warning)) => TestResult { ok: true, identity: Some(identity), warning, error: None },
            Err(e) => {
                log::warn!("Comptes : test {} : {}", definition(id).label, e.message);
                TestResult { ok: false, error: Some(e.message), ..Default::default() }
            }
        }
    }

    /// Page de création de jeton du serveur.
    pub fn token_page(&self, id: &str, fallback: bool) -> anyhow::Result<String> {
        let def = definition(id);
        let Some(s) = self.settings(id) else { anyhow::bail!("{} : renseignez d'abord l'URL.", def.label) };
        let path = if fallback { def.fallback_token_path.unwrap_or(def.token_path) } else { def.token_path };
        Ok(format!("{}{path}", s.url.trim_end_matches('/')))
    }

    /// Client HTTP du service, authentifié avec le jeton de l'utilisateur.
    pub fn client(&self, id: &str) -> Result<ServiceHttp, ServiceError> {
        let def = definition(id);
        let settings = self.settings(id).ok_or_else(|| ServiceError::new(format!("{} n'est pas configuré : Ocekour → Paramètres → Comptes.", def.label)))?;
        let raw = self
            .secrets
            .get(&secret_id(id))
            .ok_or_else(|| ServiceError::new(format!("Aucun jeton {} enregistré : Ocekour → Paramètres → Comptes.", def.label)))?;
        let auth = if def.needs_user_id {
            let v: Value = serde_json::from_str(&raw).unwrap_or_default();
            vec![
                ("X-Auth-Token".to_string(), v["token"].as_str().unwrap_or_default().to_string()),
                ("X-User-Id".to_string(), v["userId"].as_str().unwrap_or_default().to_string()),
            ]
        } else {
            vec![("Authorization".to_string(), format!("Bearer {raw}"))]
        };
        Ok(ServiceHttp { label: def.label.into(), url: settings.url, tls: settings.tls, auth, user_agent: self.user_agent.clone() })
    }
}

/// Vérifie le jeton : identité, avertissement éventuel.
async fn test_service(id: &str, http: &ServiceHttp, write: bool) -> Result<(String, Option<String>), ServiceError> {
    match id {
        "gitlab" => {
            let me = http.get("api/v4/user", vec![]).await?;
            let mut warning = None;
            // GitLab < 15.5 : pas d'introspection du jeton.
            if let Ok(token) = http.get("api/v4/personal_access_tokens/self", vec![]).await {
                let scopes: Vec<String> = token["scopes"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
                if !scopes.iter().any(|s| s == "api") {
                    warning = Some(if write {
                        format!("Portée « api » absente ({}) : les écritures échoueront. Recréez le jeton avec « api ».", scopes.join(", "))
                    } else {
                        format!("Portées : {}. « api » est recommandée.", scopes.join(", "))
                    });
                }
            }
            Ok((format!("{} (@{})", s(&me["name"]), s(&me["username"])), warning))
        }
        "jira" => {
            let me = http.get("rest/api/2/myself", vec![]).await?;
            Ok((format!("{} ({})", s(&me["displayName"]), s(&me["name"])), None))
        }
        "confluence" => {
            let me = http.get("rest/api/user/current", vec![]).await?;
            // Jeton ignoré : Confluence répond « anonyme » au lieu d'une erreur.
            if me["type"] == json!("anonymous") || me["username"].as_str().is_none_or(str::is_empty) {
                return Err(ServiceError::new("Jeton Confluence refusé : la requête est traitée comme anonyme."));
            }
            Ok((format!("{} ({})", s(&me["displayName"]), s(&me["username"])), None))
        }
        _ => {
            let me = http.get("api/v1/me", vec![]).await?;
            let name = me["name"].as_str().unwrap_or_else(|| me["username"].as_str().unwrap_or_default());
            Ok((format!("{name} (@{})", s(&me["username"])), None))
        }
    }
}

/// Texte d'une valeur JSON (vide si absente).
pub fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
