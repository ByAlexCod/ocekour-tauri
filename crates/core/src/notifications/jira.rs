//! Notifications Jira DC : tickets qui me concernent et qui ont bougé (JQL, fenêtre relative
//! sans fuseau), changelog et commentaires pour ne garder que ce que d'AUTRES ont fait.

use super::state::{EntityRef, NotifItem};
use crate::services::http::ServiceHttp;
use crate::services::s;
use serde_json::{Value, json};

pub const DEFAULT_EVENTS: [&str; 4] = ["assigned", "comment", "mention", "status"];

pub fn kind_label(kind: &str) -> String {
    match kind { "assigned" => "Assigné", "comment" => "Commentaire", "mention" => "Mention", "status" => "Changement de statut", other => other }.into()
}

/// Fenêtre relative (`-Nm`) depuis `since`, bornée à 7 jours.
pub fn relative_minutes(since: Option<&str>, now: i64) -> i64 {
    let Some(since) = since.and_then(crate::text::parse_ms) else { return 2 };
    let elapsed = (now - since) as f64 / 60_000.0;
    ((elapsed.ceil() as i64) + 2).clamp(2, 7 * 24 * 60)
}

/// Mention `[~name]` ou `[~accountid:…]` ?
pub fn mentions(text: &str, me: &str) -> bool {
    regex::Regex::new(&format!(r"\[~(?:accountid:)?{}[\]|]", regex::escape(me))).unwrap().is_match(text)
}

fn arr(v: &Value) -> &[Value] { v.as_array().map(|a| a.as_slice()).unwrap_or_default() }

/// Nouveautés depuis `since` (ISO) ; le curseur avance à `now_iso`.
pub async fn fetch(http: &ServiceHttp, since: Option<&str>, events: &[String], now_iso: &str) -> anyhow::Result<(Vec<NotifItem>, String)> {
    let me = s(&http.get("rest/api/2/myself", vec![]).await?["name"]);
    let minutes = relative_minutes(since, crate::text::parse_ms(now_iso).unwrap_or_else(crate::text::now_ms));
    let jql = format!("(assignee = currentUser() OR reporter = currentUser() OR watcher = currentUser()) AND updated >= \"-{minutes}m\" ORDER BY updated ASC");
    let search = http.post("rest/api/2/search", json!({
        "jql": jql, "maxResults": 50, "fields": ["summary", "comment", "status", "assignee", "reporter"], "expand": ["changelog"],
    })).await?;
    let cutoff = since.and_then(crate::text::parse_ms).unwrap_or_else(crate::text::now_ms);
    let want = |k: &str| events.iter().any(|e| e == k);
    let mut items = vec![];
    for issue in arr(&search["issues"]) {
        let key = s(&issue["key"]);
        let title = format!("{key} « {} »", s(&issue["fields"]["summary"]));
        let url = http.web(&format!("browse/{key}"));
        let entity = EntityRef { source: "jira".into(), r#type: "ticket".into(), key: key.clone(), title: title.clone(), url: Some(url.clone()), participants: None };
        let mut add = |kind: &str, id: String, at: &Value, actor: Option<String>, body: Option<String>| {
            let Some(at) = at.as_str() else { return };
            if crate::text::parse_ms(at).is_none_or(|t| t <= cutoff) { return; }
            items.push(NotifItem {
                source: "jira".into(), kind: kind.into(), title: title.clone(), url: Some(url.clone()), actor,
                dedup_key: format!("jira:{key}:{id}"), entity: Some(entity.clone()), body, at: Some(at.into()),
            });
        };
        for h in arr(&issue["changelog"]["histories"]) {
            let author = h["author"]["name"].as_str().map(str::to_string);
            if author.as_deref() == Some(me.as_str()) { continue; }
            for it in arr(&h["items"]) {
                if it["field"] == "assignee" && it["to"].as_str() == Some(me.as_str()) && want("assigned") {
                    add("assigned", format!("h{}", s(&h["id"])), &h["created"], author.clone(), None);
                } else if it["field"] == "status" && want("status") {
                    add("status", format!("h{}", s(&h["id"])), &h["created"], author.clone(), Some(format!("{} → {}", s(&it["fromString"]), s(&it["toString"]))));
                }
            }
        }
        for c in arr(&issue["fields"]["comment"]["comments"]) {
            let author = c["author"]["name"].as_str().map(str::to_string);
            if author.as_deref() == Some(me.as_str()) { continue; }
            let body = c["body"].as_str().unwrap_or_default().to_string();
            if want("mention") && mentions(&body, &me) {
                add("mention", format!("c{}", s(&c["id"])), &c["created"], author, Some(body));
            } else if want("comment") {
                add("comment", format!("c{}", s(&c["id"])), &c["created"], author, Some(body));
            }
        }
    }
    Ok((items, now_iso.into()))
}
