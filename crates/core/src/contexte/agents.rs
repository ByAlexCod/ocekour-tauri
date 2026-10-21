//! Auto-configuration du serveur MCP d'Ocekour chez les agents locaux (Qwen Code :
//! settings.json ; Mistral Vibe : config.toml). Fusion idempotente, sauvegarde `.bak`.

use serde::Serialize;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const MCP_ENTRY: &str = "ocekour";

pub struct McpTarget {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentState {
    pub tool: &'static str,
    pub present: bool,
    pub configured: bool,
    pub file: String,
}

fn qwen_file(home: &Path) -> PathBuf { home.join(".qwen").join("settings.json") }
fn vibe_file(home: &Path) -> PathBuf { home.join(".vibe").join("config.toml") }

fn read_jsonc(file: &Path) -> anyhow::Result<Value> {
    let Ok(raw) = std::fs::read_to_string(file) else { return Ok(Value::Object(Map::new())) };
    if let Ok(v) = serde_json::from_str(&raw) { return Ok(v); }
    let block = regex::Regex::new(r"(?s)/\*.*?\*/").unwrap();
    let line = regex::Regex::new(r"(?m)(^|[^:])//.*$").unwrap();
    Ok(serde_json::from_str(&line.replace_all(&block.replace_all(&raw, ""), "$1"))?)
}

fn qwen_has_ocekour(home: &Path) -> bool { read_jsonc(&qwen_file(home)).is_ok_and(|j| !j["mcpServers"][MCP_ENTRY].is_null()) }

fn is_ocekour_block(b: &str) -> bool {
    regex::Regex::new(r"\[\[\s*mcp_servers\s*\]\]").unwrap().is_match(b) && regex::Regex::new(r#"name\s*=\s*["']ocekour["']"#).unwrap().is_match(b)
}

/// Blocs TOML (coupés avant chaque `[` en début de ligne).
fn blocks(toml: &str) -> Vec<&str> {
    let mut cuts = vec![0];
    cuts.extend(toml.match_indices("\n[").map(|(i, _)| i + 1));
    cuts.push(toml.len());
    cuts.windows(2).map(|w| toml[w[0]..w[1]].trim_end_matches('\n')).collect()
}

fn vibe_has_ocekour(home: &Path) -> bool {
    std::fs::read_to_string(vibe_file(home)).is_ok_and(|t| blocks(&t).into_iter().any(is_ocekour_block))
}

pub fn agent_states(home: &Path) -> Vec<AgentState> {
    vec![
        AgentState { tool: "qwen", present: home.join(".qwen").exists(), configured: qwen_has_ocekour(home), file: qwen_file(home).to_string_lossy().into() },
        AgentState { tool: "vibe", present: home.join(".vibe").exists(), configured: vibe_has_ocekour(home), file: vibe_file(home).to_string_lossy().into() },
    ]
}

/// Écrit `content` après avoir sauvegardé l'ancien fichier en `.bak`.
fn write_with_backup(file: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() { std::fs::create_dir_all(dir)?; }
    if file.exists() { let _ = std::fs::copy(file, format!("{}.bak", file.display())); }
    let tmp = PathBuf::from(format!("{}.{}.tmp", file.display(), std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

/// Ajoute (`add`) ou retire Ocekour de la config d'un agent (`qwen` ou `vibe`).
pub fn configure_agent(tool: &str, add: bool, target: &McpTarget, home: &Path) -> anyhow::Result<()> {
    if tool == "qwen" {
        let file = qwen_file(home);
        let mut json = read_jsonc(&file)?;
        if !json.is_object() { json = json!({}); }
        if !json["mcpServers"].is_object() { json["mcpServers"] = json!({}); }
        let servers = json["mcpServers"].as_object_mut().unwrap();
        if add {
            servers.insert(MCP_ENTRY.into(), json!({ "httpUrl": target.url, "headers": { "Authorization": format!("Bearer {}", target.token), "Accept": "application/json, text/event-stream" } }));
        } else {
            servers.remove(MCP_ENTRY);
        }
        write_with_backup(&file, &format!("{}\n", serde_json::to_string_pretty(&json)?))?;
        log::info!("Contexte : Qwen {} ({})", if add { "configuré" } else { "nettoyé" }, file.display());
    } else {
        let file = vibe_file(home);
        let current = std::fs::read_to_string(&file).unwrap_or_default();
        // Blocs Ocekour retirés, avec leurs sous-tables (`[mcp_servers.auth]`).
        let mut dropping = false;
        let kept = blocks(&current)
            .into_iter()
            .filter(|b| {
                if is_ocekour_block(b) { dropping = true; return false; }
                if dropping && b.starts_with("[mcp_servers.") { return false; }
                dropping = false;
                true
            })
            .collect::<Vec<_>>()
            .join("\n");
        let kept = kept.trim_end();
        let next = if add {
            let block = format!(
                "[[mcp_servers]]\nname = \"{MCP_ENTRY}\"\ntransport = \"streamable-http\"\nurl = \"{}\"\n\n[mcp_servers.auth]\ntype = \"static\"\nheader = \"Authorization\"\nvalue = \"Bearer {}\"",
                target.url, target.token
            );
            if kept.is_empty() { format!("{block}\n") } else { format!("{kept}\n\n{block}\n") }
        } else if kept.is_empty() {
            String::new()
        } else {
            format!("{kept}\n")
        };
        write_with_backup(&file, &next)?;
        log::info!("Contexte : Vibe {} ({})", if add { "configuré" } else { "nettoyé" }, file.display());
    }
    Ok(())
}
