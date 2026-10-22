//! Modifications de config.yaml depuis l'interface ou le MCP : préférences, notifications,
//! contexte IA, favoris, profils « en un clic », filtres enregistrés. Seules les clés
//! concernées sont réécrites (commentaires conservés).

use crate::config::{ConfigState, ConfigStore, PREFERENCE_KEYS, stored_filter_rule_schema};
use crate::schema::{self as z, S};
use crate::ypath;
use serde_json::{Map, Value, json};

fn issues(e: Vec<crate::schema::Issue>) -> anyhow::Error { anyhow::anyhow!(e[0].describe()) }

/// Préférences (thème, tampon, serveur MCP…) ; `null` ou `""` retire la clé.
pub fn set_preferences(config: &ConfigStore, patch: &Value) -> anyhow::Result<ConfigState> {
    let entries = patch.as_object().cloned().ok_or_else(|| anyhow::anyhow!("Préférences : objet attendu"))?;
    if let Some((k, _)) = entries.iter().find(|(k, _)| !PREFERENCE_KEYS.contains(&k.as_str())) {
        anyhow::bail!("Préférence inconnue : {k}");
    }
    // La validation complète du document est faite par `edit` avant écriture.
    config.edit(move |doc, _| {
        for (key, value) in entries {
            if value.is_null() || value == json!("") { doc.delete(&ypath![key])?; } else { doc.set(&ypath![key], &value)?; }
        }
        Ok(())
    })
}

fn source_patch(with_events: bool) -> S {
    let mut fields = vec![("enabled", z::boolean().optional())];
    if with_events { fields.push(("events", z::array(z::string()).optional())); }
    z::object(fields).optional()
}

fn merge(target: &mut Value, patch: &Value) {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p { t.insert(k.clone(), v.clone()); }
    }
}

/// Réglages des notifications (fusion partielle).
pub fn set_notifications(config: &ConfigStore, patch: &Value) -> anyhow::Result<ConfigState> {
    let schema = z::object(vec![
        ("enabled", z::boolean().optional()),
        ("intervalSeconds", z::int().min(30).max(3_600).optional()),
        ("target", z::string().max(200).optional()),
        ("desktop", z::boolean().optional()),
        ("sources", z::object(crate::config::NOTIFICATION_SOURCES.iter().map(|s| (*s, source_patch(true))).collect()).optional()),
    ]);
    let parsed = schema.parse(patch).map_err(issues)?;
    let mut next = config.state().config["notifications"].clone();
    for k in ["enabled", "intervalSeconds", "target", "desktop"] {
        if let Some(v) = parsed.get(k) { next[k] = v.clone(); }
    }
    if let Some(sources) = parsed.get("sources").and_then(|s| s.as_object()) {
        for (name, p) in sources { merge(&mut next["sources"][name], p); }
    }
    config.edit(move |doc, _| Ok(doc.set(&ypath!["notifications"], &next)?))
}

/// Fichiers visibles par les agents (liste remplacée ; chemins absolus, fichiers ou motifs).
pub fn set_fichiers(config: &ConfigStore, patch: &Value) -> anyhow::Result<ConfigState> {
    let schema = z::object(vec![("agents", z::array(z::string().min(1)).max(100).optional())]);
    let parsed = schema.parse(patch).map_err(issues)?;
    let mut next = config.state().config["fichiers"].clone();
    if let Some(agents) = parsed.get("agents").and_then(|a| a.as_array()) {
        let mut list: Vec<Value> = vec![];
        for a in agents {
            let path = a.as_str().unwrap_or_default().trim().to_string();
            crate::streams::file::FileTarget::parse(&path).map_err(|e| anyhow::anyhow!("{path} : {}", e.message))?;
            if !list.contains(&json!(path)) { list.push(json!(path)); }
        }
        next["agents"] = Value::Array(list);
    }
    config.edit(move |doc, _| Ok(doc.set(&ypath!["fichiers"], &next)?))
}

/// Réglages du contexte IA (fusion partielle ; `tls.caFile: null` retire le fichier).
pub fn set_contexte(config: &ConfigStore, patch: &Value) -> anyhow::Result<ConfigState> {
    let schema = z::object(vec![
        ("enabled", z::boolean().optional()),
        ("intervalSeconds", z::int().min(30).max(3_600).optional()),
        ("llm", z::enumeration(&["auto", "qwen", "vibe", "off"]).optional()),
        ("tls", z::object(vec![("insecure", z::boolean().optional()), ("caFile", z::string().nullable().optional())]).optional()),
        ("sources", z::object(crate::config::CONTEXT_SOURCES.iter().map(|s| (*s, source_patch(false))).collect()).optional()),
    ]);
    let parsed = schema.parse(patch).map_err(issues)?;
    let current = config.state().config["contexte"].clone();
    let mut next = current.clone();
    for k in ["enabled", "intervalSeconds", "llm"] {
        if let Some(v) = parsed.get(k) { next[k] = v.clone(); }
    }
    if let Some(tls) = parsed.get("tls") {
        let mut t = Map::new();
        t.insert("insecure".into(), tls.get("insecure").cloned().unwrap_or_else(|| current["tls"]["insecure"].clone()));
        match tls.get("caFile") {
            Some(Value::Null) => {}
            Some(v) => { t.insert("caFile".into(), v.clone()); }
            None => {
                if let Some(c) = current["tls"].get("caFile") { t.insert("caFile".into(), c.clone()); }
            }
        }
        next["tls"] = Value::Object(t);
    }
    if let Some(sources) = parsed.get("sources").and_then(|s| s.as_object()) {
        for (name, p) in sources { merge(&mut next["sources"][name], p); }
    }
    config.edit(move |doc, _| Ok(doc.set(&ypath!["contexte"], &next)?))
}

/// Épingle ou désépingle un namespace / workload.
pub fn toggle_favorite(config: &ConfigStore, key: &str) -> anyhow::Result<ConfigState> {
    if key.is_empty() { anyhow::bail!("Favori vide."); }
    let key = key.to_string();
    config.edit(move |doc, current| {
        let mut next = current.favorites.clone();
        if next.contains(&key) { next.retain(|f| *f != key); } else { next.push(key); next.sort(); }
        Ok(doc.set(&ypath!["favorites"], &json!(next))?)
    })
}

pub fn profile_edit_schema() -> S {
    z::disc("op", vec![
        ("toggleExpand", z::object(vec![("op", z::literal(json!("toggleExpand"))), ("path", z::string().min(1))])),
        ("toggleColumn", z::object(vec![("op", z::literal(json!("toggleColumn"))), ("path", z::string().min(1))])),
        ("toggleWrap", z::object(vec![("op", z::literal(json!("toggleWrap"))), ("path", z::string().min(1))])),
        (
            "setField",
            z::object(vec![
                ("op", z::literal(json!("setField"))),
                ("role", z::enumeration(&["time", "level", "message"])),
                ("path", z::string().min(1).nullable()),
            ]),
        ),
        ("setParseNested", z::object(vec![("op", z::literal(json!("setParseNested"))), ("value", z::boolean())])),
    ])
}

/// Modification « en un clic » du profil `match` (créé en tête de liste s'il n'existe pas).
pub fn edit_profile(config: &ConfigStore, match_: &str, edit: &Value) -> anyhow::Result<ConfigState> {
    if match_.is_empty() { anyhow::bail!("match : Trop petit : chaîne doit avoir >=1 caractères"); }
    let edit = profile_edit_schema().parse(edit).map_err(issues)?;
    let match_ = match_.to_string();
    config.edit(move |doc, current| {
        let profile = current.profiles.iter().find(|p| p.match_ == match_).cloned();
        let index = match current.profiles.iter().position(|p| p.match_ == match_) {
            Some(i) => i,
            None => {
                doc.unshift(&ypath!["profiles"], &json!({ "match": match_ }))?;
                0
            }
        };
        let at = |keys: &[&str]| {
            let mut p = ypath!["profiles", index];
            p.extend(keys.iter().map(|k| crate::yaml::P::K(k.to_string())));
            p
        };
        let path = edit["path"].as_str().unwrap_or_default().to_string();
        let mut toggle = |key: &str, current: Vec<String>| -> anyhow::Result<()> {
            let mut next = current;
            if next.contains(&path) { next.retain(|p| *p != path); } else { next.push(path.clone()); }
            if next.is_empty() { doc.delete(&at(&[key]))?; } else { doc.set(&at(&[key]), &json!(next))?; }
            Ok(())
        };
        match edit["op"].as_str().unwrap() {
            "toggleExpand" => toggle("expand", profile.map(|p| p.expand).unwrap_or_default()),
            "toggleColumn" => toggle("columns", profile.map(|p| p.columns).unwrap_or_default()),
            "toggleWrap" => toggle("wrap", profile.map(|p| p.wrap).unwrap_or_default()),
            "setParseNested" => Ok(doc.set(&at(&["parseNested"]), &edit["value"])?),
            _ => {
                let role = edit["role"].as_str().unwrap();
                if edit["path"].is_null() { doc.delete(&at(&["fields", role]))?; } else { doc.set(&at(&["fields", role]), &edit["path"])?; }
                if doc.get(&at(&["fields"])).is_some_and(|f| f.as_object().is_some_and(|m| m.is_empty())) {
                    doc.delete(&at(&["fields"]))?;
                }
                Ok(())
            }
        }
    })
}

fn preset_name(v: &Value) -> anyhow::Result<String> {
    z::string().trim().min(1).max(100).parse(v).map(|v| v.as_str().unwrap().to_string()).map_err(issues)
}

fn write_presets(config: &ConfigStore, mut next: Vec<Value>) -> anyhow::Result<ConfigState> {
    next.sort_by(|a, b| crate::text::locale_cmp(a["name"].as_str().unwrap_or(""), b["name"].as_str().unwrap_or("")));
    config.edit(move |doc, _| {
        if next.is_empty() { doc.delete(&ypath!["filterPresets"])?; } else { doc.set(&ypath!["filterPresets"], &Value::Array(next))?; }
        Ok(())
    })
}

fn current_presets(config: &ConfigStore) -> Vec<Value> {
    config.state().config["filterPresets"].as_array().cloned().unwrap_or_default()
}

/// Enregistre (ou remplace, par nom) un filtre nommé.
pub fn save_filter_preset(config: &ConfigStore, name: &Value, rules: &Value) -> anyhow::Result<ConfigState> {
    let name = preset_name(name)?;
    let rules = z::array(stored_filter_rule_schema()).parse(rules).map_err(issues)?;
    let mut next: Vec<Value> = current_presets(config).into_iter().filter(|p| p["name"] != json!(name)).collect();
    next.push(json!({ "name": name, "rules": rules }));
    write_presets(config, next)
}

pub fn delete_filter_preset(config: &ConfigStore, name: &Value) -> anyhow::Result<ConfigState> {
    let name = preset_name(name)?;
    write_presets(config, current_presets(config).into_iter().filter(|p| p["name"] != json!(name)).collect())
}

/// Renomme un filtre ; refusé si le nouveau nom est déjà pris.
pub fn rename_filter_preset(config: &ConfigStore, name: &Value, to: &Value) -> anyhow::Result<ConfigState> {
    let (name, to) = (preset_name(name)?, preset_name(to)?);
    let presets = current_presets(config);
    if to != name && presets.iter().any(|p| p["name"] == json!(to)) {
        anyhow::bail!("Un filtre nommé « {to} » existe déjà.");
    }
    write_presets(config, presets.into_iter().map(|mut p| { if p["name"] == json!(name) { p["name"] = json!(to); } p }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn store(text: &str) -> (tempfile::TempDir, Arc<ConfigStore>) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.yaml");
        std::fs::write(&file, text).unwrap();
        let s = ConfigStore::new(&file);
        (dir, s)
    }

    fn text(s: &ConfigStore) -> String { std::fs::read_to_string(&s.file).unwrap() }

    #[test]
    fn profils_en_un_clic() {
        let (_d, s) = store("# mes profils\nprofiles:\n  - match: '*' # défaut\n    parseNested: false\n");
        let m = "prod/app/api";
        edit_profile(&s, m, &json!({ "op": "toggleExpand", "path": "stack_trace" })).unwrap();
        edit_profile(&s, m, &json!({ "op": "toggleColumn", "path": "traceId" })).unwrap();
        edit_profile(&s, m, &json!({ "op": "setField", "role": "message", "path": "msg" })).unwrap();
        edit_profile(&s, m, &json!({ "op": "setParseNested", "value": true })).unwrap();
        edit_profile(&s, m, &json!({ "op": "toggleWrap", "path": "@message" })).unwrap();
        let p = &s.config().profiles[0];
        assert_eq!(p.match_, m);
        assert_eq!(p.expand, vec!["stack_trace"]);
        assert_eq!(p.columns, vec!["traceId"]);
        assert_eq!(p.fields.message.as_deref(), Some("msg"));
        assert_eq!(p.parse_nested, Some(true));
        assert_eq!(p.wrap, vec!["@message"]);
        assert_eq!(s.config().profiles[1].match_, "*");
        assert!(text(&s).contains("# mes profils") && text(&s).contains("# défaut"));
        // Retours en arrière : clés retirées.
        edit_profile(&s, m, &json!({ "op": "toggleExpand", "path": "stack_trace" })).unwrap();
        edit_profile(&s, m, &json!({ "op": "setField", "role": "message", "path": null })).unwrap();
        let t = text(&s);
        assert!(!t.contains("expand") && !t.contains("fields"), "{t}");
        assert!(edit_profile(&s, m, &json!({ "op": "inconnu" })).is_err());
    }

    #[test]
    fn filtres_enregistres() {
        let (_d, s) = store("");
        let rules = json!([{ "field": "http.path", "value": "/health" }]);
        save_filter_preset(&s, &json!("Sans santé"), &rules).unwrap();
        save_filter_preset(&s, &json!("Autre"), &json!([])).unwrap();
        let presets = &s.state().config["filterPresets"];
        assert_eq!(presets[0]["name"], json!("Autre"));
        assert_eq!(presets[1]["rules"][0], json!({ "enabled": true, "field": "http.path", "mode": "contains", "value": "/health", "caseSensitive": false, "action": "hide" }));
        assert!(rename_filter_preset(&s, &json!("Autre"), &json!("Sans santé")).unwrap_err().to_string().contains("existe déjà"));
        rename_filter_preset(&s, &json!("Autre"), &json!("Zéro")).unwrap();
        delete_filter_preset(&s, &json!("Sans santé")).unwrap();
        delete_filter_preset(&s, &json!("Zéro")).unwrap();
        assert!(!text(&s).contains("filterPresets"));
        assert!(save_filter_preset(&s, &json!("  "), &json!([])).is_err());
    }

    #[test]
    fn preferences_favoris_notifications_contexte() {
        let (_d, s) = store("theme: dark # sombre\n");
        set_preferences(&s, &json!({ "theme": "light", "mcpServer": true })).unwrap();
        assert!(text(&s).contains("theme: light # sombre"));
        assert!(set_preferences(&s, &json!({ "inconnue": 1 })).unwrap_err().to_string().contains("Préférence inconnue"));
        assert!(set_preferences(&s, &json!({ "bufferSize": 3 })).is_err());
        set_preferences(&s, &json!({ "updateShare": "" })).unwrap();
        toggle_favorite(&s, "c/k/ns").unwrap();
        toggle_favorite(&s, "a/b/c").unwrap();
        assert_eq!(s.config().favorites, vec!["a/b/c", "c/k/ns"]);
        toggle_favorite(&s, "a/b/c").unwrap();
        assert_eq!(s.config().favorites, vec!["c/k/ns"]);
        set_notifications(&s, &json!({ "enabled": true, "sources": { "jira": { "enabled": false } } })).unwrap();
        let n = &s.state().config["notifications"];
        assert_eq!((n["enabled"].clone(), n["sources"]["jira"]["enabled"].clone(), n["sources"]["gitlab"]["enabled"].clone()), (json!(true), json!(false), json!(true)));
        assert!(set_notifications(&s, &json!({ "intervalSeconds": 5 })).is_err());
        set_contexte(&s, &json!({ "llm": "off", "tls": { "caFile": "C:\\ca.pem" } })).unwrap();
        assert_eq!(s.state().config["contexte"]["tls"], json!({ "insecure": false, "caFile": "C:\\ca.pem" }));
        set_contexte(&s, &json!({ "tls": { "caFile": null } })).unwrap();
        assert_eq!(s.state().config["contexte"]["tls"], json!({ "insecure": false }));
        assert_eq!(s.config().contexte.llm, "off");
    }
}
