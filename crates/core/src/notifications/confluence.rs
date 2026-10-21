//! Notifications Confluence DC : CQL avec fenêtre relative `now("-Nm")` (sans fuseau). Pages /
//! commentaires où je suis mentionné, et contenus que je suis.

use super::jira::relative_minutes;
use super::state::{EntityRef, NotifItem};
use crate::services::http::ServiceHttp;
use crate::services::s;
use serde_json::json;

pub const DEFAULT_EVENTS: [&str; 3] = ["mention", "comment", "update"];

pub fn kind_label(kind: &str) -> String {
    match kind { "mention" => "Mention", "comment" => "Commentaire", "update" => "Page modifiée", other => other }.into()
}

pub async fn fetch(http: &ServiceHttp, since: Option<&str>, events: &[String], now_iso: &str) -> anyhow::Result<(Vec<NotifItem>, String)> {
    let me = s(&http.get("rest/api/user/current", vec![]).await?["username"]);
    let minutes = relative_minutes(since, crate::text::parse_ms(now_iso).unwrap_or_else(crate::text::now_ms));
    let window = format!("now(\"-{minutes}m\")");
    let cutoff = since.and_then(crate::text::parse_ms).unwrap_or_else(crate::text::now_ms);
    let want = |k: &str| events.iter().any(|e| e == k);
    let mut by_key: Vec<NotifItem> = vec![];

    let mut queries: Vec<(String, bool)> = vec![];
    if want("mention") { queries.push((format!("mention = currentUser() AND lastModified > {window}"), true)); }
    if want("comment") || want("update") { queries.push((format!("watcher = currentUser() AND lastModified > {window}"), false)); }
    for (cql, forced_mention) in queries {
        let r = http.get("rest/api/content/search", vec![("cql", json!(cql)), ("limit", json!(50)), ("expand", json!("version,space,history"))]).await?;
        for c in r["results"].as_array().map(|a| a.as_slice()).unwrap_or_default() {
            let version = &c["version"];
            let author = version["by"]["username"].as_str().map(str::to_string);
            if author.as_deref() == Some(me.as_str()) { continue; }
            if let Some(when) = version["when"].as_str() {
                if crate::text::parse_ms(when).is_some_and(|t| t <= cutoff) { continue; }
            }
            let kind = if forced_mention { "mention" } else if c["type"] == "comment" { "comment" } else { "update" };
            if !want(kind) { continue; }
            let key = format!("confluence:{}@{}", s(&c["id"]), s(&version["number"]));
            let title = format!("{}{}", c["title"].as_str().unwrap_or("Confluence"), c["space"]["key"].as_str().map(|k| format!(" ({k})")).unwrap_or_default());
            let url = match c["_links"]["webui"].as_str() { Some(w) => http.web(w), None => http.web(&format!("pages/viewpage.action?pageId={}", s(&c["id"]))) };
            let page_id = if c["type"] == "comment" && !c["container"]["id"].is_null() { s(&c["container"]["id"]) } else { s(&c["id"]) };
            let item = NotifItem {
                source: "confluence".into(), kind: kind.into(), title: title.clone(), url: Some(url.clone()), actor: author, dedup_key: key.clone(),
                entity: Some(EntityRef { source: "confluence".into(), r#type: "page".into(), key: page_id, title, url: Some(url), participants: None }),
                body: None, at: version["when"].as_str().map(str::to_string),
            };
            // Une mention l'emporte sur « page modifiée » pour le même contenu.
            match by_key.iter_mut().find(|i| i.dedup_key == key) {
                None => by_key.push(item),
                Some(existing) if forced_mention && existing.kind != "mention" => *existing = item,
                _ => {}
            }
        }
    }
    Ok((by_key, now_iso.into()))
}
