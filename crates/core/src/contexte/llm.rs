//! Curation par un LLM local, réutilisant la configuration de Qwen Code ou Mistral Vibe. Aucune
//! clé n'est stockée par Ocekour. Jamais bloquant : en cas d'échec, on garde le texte brut.

use crate::config::Tls;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    /// Point de terminaison OpenAI-compatible, terminé par `/v1`.
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    /// qwen | vibe
    pub source: &'static str,
}

/// JSON tolérant (commentaires de style C).
fn parse_jsonc(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let block = regex::Regex::new(r"(?s)/\*.*?\*/").unwrap();
        let line = regex::Regex::new(r"(?m)(^|[^:])//.*$").unwrap();
        let stripped = line.replace_all(&block.replace_all(text, ""), "$1").into_owned();
        serde_json::from_str(&stripped).ok()
    })
}

/// `$VAR` / `${VAR}` → variables d'environnement.
fn expand_env(value: Option<&str>, env: &HashMap<String, String>) -> Option<String> {
    let v = value?;
    let re = regex::Regex::new(r"\$\{?(\w+)\}?").unwrap();
    Some(re.replace_all(v, |c: &regex::Captures| env.get(&c[1]).cloned().unwrap_or_default()).into_owned())
}

fn read_dot_env(file: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(file) else { return out };
    let re = regex::Regex::new(r"^\s*([\w.]+)\s*=\s*(.*?)\s*$").unwrap();
    for line in text.lines() {
        if let Some(c) = re.captures(line) {
            out.insert(c[1].to_string(), c[2].trim_start_matches(['"', '\'']).trim_end_matches(['"', '\'']).to_string());
        }
    }
    out
}

fn normalize_base(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if regex::Regex::new(r"/v\d+$").unwrap().is_match(trimmed) { trimmed.into() } else { format!("{trimmed}/v1") }
}

fn st(v: &Value) -> Option<&str> { v.as_str().filter(|s| !s.is_empty()) }

/// Config Qwen Code (`~/.qwen/settings.json`, puis `~/.qwen/.env`).
fn read_qwen(home: &Path, env: &HashMap<String, String>) -> Option<LlmConfig> {
    let dir = home.join(".qwen");
    let json = std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|t| {
        let parsed = parse_jsonc(&t);
        if parsed.is_none() { log::warn!("Contexte : settings.json de Qwen illisible"); }
        parsed
    }).unwrap_or(Value::Object(Map::new()));
    let mut merged = env.clone();
    merged.extend(read_dot_env(&dir.join(".env")));
    if let Some(block) = json["env"].as_object() {
        for (k, v) in block { if let Some(v) = v.as_str() { merged.insert(k.clone(), v.into()); } }
    }
    let model_name = st(&json["model"]["name"]).or(st(&json["model.name"])).or(st(&json["model"])).map(str::to_string)
        .or_else(|| expand_env(merged.get("OPENAI_MODEL").map(String::as_str), &merged));
    let mut base_url = None;
    let mut api_key = None;
    let mut model = model_name.clone();
    let selected = st(&json["security"]["auth"]["selectedType"]).unwrap_or("openai");
    if let Some(providers) = json["modelProviders"][selected].as_array().filter(|p| !p.is_empty()) {
        let entry = providers.iter().find(|p| model_name.is_some() && p["id"].as_str() == model_name.as_deref()).unwrap_or(&providers[0]);
        base_url = expand_env(st(&entry["baseUrl"]), &merged);
        model = st(&entry["id"]).map(str::to_string).or(model_name);
        api_key = expand_env(st(&entry["apiKey"]), &merged).or_else(|| st(&entry["envKey"]).and_then(|k| merged.get(k).cloned()));
    }
    let base_url = base_url
        .or_else(|| expand_env(st(&json["model"]["baseUrl"]).or(st(&json["model.baseUrl"])).or(st(&json["baseUrl"])), &merged))
        .or_else(|| expand_env(merged.get("OPENAI_BASE_URL").map(String::as_str), &merged))
        .filter(|b| !b.is_empty())?;
    let api_key = api_key
        .or_else(|| expand_env(st(&json["model"]["apiKey"]).or(st(&json["model.apiKey"])), &merged))
        .or_else(|| merged.get("OPENAI_API_KEY").cloned());
    let model = model.filter(|m| !m.is_empty())?;
    Some(LlmConfig { base_url: normalize_base(&base_url), model, api_key, source: "qwen" })
}

/// Première valeur `clé = "valeur"` d'un texte TOML.
fn toml_value(toml: &str, key: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r#"(?m)^\s*{key}\s*=\s*["']?([^"'\n#]+)["']?"#)).unwrap();
    re.captures(toml).map(|c| c[1].trim().to_string()).filter(|v| !v.is_empty())
}

/// Config Mistral Vibe (`~/.vibe/config.toml` + `~/.vibe/.env`).
fn read_vibe(home: &Path, env: &HashMap<String, String>) -> Option<LlmConfig> {
    let dir = home.join(".vibe");
    let toml = std::fs::read_to_string(dir.join("config.toml")).ok()?;
    let mut merged = read_dot_env(&dir.join(".env"));
    merged.extend(env.clone());
    let active = toml_value(&toml, "active_model");
    let models = regex::Regex::new(r"\[\[\s*models\s*\]\]").unwrap();
    let block = active.as_ref().and_then(|name| {
        let starts: Vec<usize> = toml.match_indices("\n[[").map(|(i, _)| i + 1).collect();
        let mut bounds = vec![0];
        bounds.extend(starts);
        bounds.push(toml.len());
        bounds.windows(2).map(|w| &toml[w[0]..w[1]]).find(|b| models.is_match(b) && (toml_value(b, "name").as_ref() == Some(name) || toml_value(b, "id").as_ref() == Some(name))).map(str::to_string)
    }).unwrap_or_else(|| toml.clone());
    let model = toml_value(&block, "model").or(active)?;
    let base_url = toml_value(&block, "base_url")?;
    let api_key = match toml_value(&block, "api_key_env") { Some(k) => merged.get(&k).cloned(), None => merged.get("MISTRAL_API_KEY").cloned() };
    Some(LlmConfig { base_url: normalize_base(&base_url), model, api_key, source: "vibe" })
}

/// Config LLM selon le mode (`auto` : Qwen puis Vibe).
pub fn discover(mode: &str, home: &Path, env: &HashMap<String, String>) -> Option<LlmConfig> {
    match mode {
        "off" => None,
        "qwen" => read_qwen(home, env),
        "vibe" => read_vibe(home, env),
        _ => read_qwen(home, env).or_else(|| read_vibe(home, env)),
    }
}

/// Avec le dossier personnel et l'environnement du processus.
pub fn discover_here(mode: &str) -> Option<LlmConfig> {
    let home = dirs::home_dir()?;
    discover(mode, &home, &std::env::vars().collect())
}

const SYSTEM: &str = "Tu condenses une nouveauté professionnelle (commentaire, message, changement) en 1 à 3 lignes en français. Garde les identifiants, décisions et demandes. N’invente rien, n’ajoute aucune formule. Réponds seulement par le résumé.";

/// Résume le texte d'un événement via le LLM ; `None` si absent, en erreur ou trop lent.
pub struct Curator {
    cfg: Option<LlmConfig>,
    tls: Tls,
}

impl Curator {
    pub fn new(cfg: Option<LlmConfig>, tls: Tls) -> Self { Curator { cfg, tls } }
    pub fn available(&self) -> bool { self.cfg.is_some() }
    pub fn describe(&self) -> Option<String> { self.cfg.as_ref().map(|c| format!("{} · {}", c.source, c.model)) }

    pub async fn curate(&self, text: &str, context: &str) -> Option<String> {
        let cfg = self.cfg.as_ref()?;
        if text.trim().is_empty() { return None; }
        let result = async {
            let client = crate::net::http_client(&self.tls)?;
            let content: String = format!("{context}\n\n{text}").chars().take(6_000).collect();
            let mut req = client
                .post(format!("{}/chat/completions", cfg.base_url))
                .timeout(Duration::from_secs(15))
                .json(&json!({ "model": cfg.model, "temperature": 0, "max_tokens": 200, "messages": [{ "role": "system", "content": SYSTEM }, { "role": "user", "content": content }] }));
            if let Some(k) = &cfg.api_key { req = req.bearer_auth(k); }
            let res = req.send().await?;
            if !res.status().is_success() { anyhow::bail!("HTTP {}", res.status().as_u16()); }
            let v: Value = res.json().await?;
            Ok::<_, anyhow::Error>(v["choices"][0]["message"]["content"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        }
        .await;
        match result {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Contexte : curation LLM ignorée : {e}");
                None
            }
        }
    }
}
