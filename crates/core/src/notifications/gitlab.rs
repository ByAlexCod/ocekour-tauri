//! Notifications GitLab : la boîte To-Do (`/api/v4/todos`), déjà qualifiée par action, et les
//! commentaires de MR qui concernent l'utilisateur.

use super::state::{EntityRef, NotifItem};
use crate::services::http::ServiceHttp;
use crate::services::s;
use serde_json::{Value, json};

const TODO_EVENTS: [&str; 7] = ["assigned", "review_requested", "mentioned", "directly_addressed", "build_failed", "approval_required", "unmergeable"];
pub const NOTE_EVENTS: [&str; 2] = ["mr_comment", "reply"];

pub fn default_events() -> Vec<&'static str> { TODO_EVENTS.iter().chain(NOTE_EVENTS.iter()).copied().collect() }

pub fn kind_label(kind: &str) -> String {
    match kind {
        "assigned" => "Assigné",
        "review_requested" => "Revue demandée",
        "mentioned" | "directly_addressed" => "Mention",
        "build_failed" => "Pipeline en échec",
        "approval_required" => "Approbation demandée",
        "unmergeable" => "MR non mergeable",
        "marked" => "À faire",
        "member_access_requested" => "Accès demandé",
        "mr_comment" => "Commentaire sur votre MR",
        "reply" => "Réponse à votre commentaire",
        other => other,
    }
    .into()
}

fn arr(v: &Value) -> &[Value] { v.as_array().map(|a| a.as_slice()).unwrap_or_default() }
fn opt(v: &Value) -> Option<String> { v.as_str().map(str::to_string) }

/// « groupe/app !42 « Titre » ».
fn target_title(todo: &Value) -> String {
    let project = s(&todo["project"]["path_with_namespace"]);
    let target = &todo["target"];
    let sign = match todo["target_type"].as_str() { Some("MergeRequest") => "!", Some("Issue") => "#", _ => "" };
    let reference = if target["iid"].is_null() { String::new() } else { format!(" {sign}{}", s(&target["iid"])) };
    let name = target["title"].as_str().map(|t| format!(" « {t} »")).unwrap_or_default();
    let title = format!("{project}{reference}{name}").trim().to_string();
    if !title.is_empty() { title } else { todo["body"].as_str().filter(|b| !b.is_empty()).unwrap_or("GitLab").to_string() }
}

fn todo_entity(todo: &Value, title: &str, url: Option<&str>) -> Option<EntityRef> {
    let project = todo["project"]["path_with_namespace"].as_str()?;
    let iid = &todo["target"]["iid"];
    if iid.is_null() { return None; }
    let is_mr = todo["target_type"] == "MergeRequest";
    if !is_mr && todo["target_type"] != "Issue" { return None; }
    Some(EntityRef {
        source: "gitlab".into(),
        r#type: if is_mr { "merge_request" } else { "issue" }.into(),
        key: format!("{project}{}{}", if is_mr { "!" } else { "#" }, s(iid)),
        title: title.into(),
        url: url.map(Into::into),
        participants: None,
    })
}

/// To-do plus récents que `last_id`, filtrés par type d'événement.
pub async fn fetch_todos(http: &ServiceHttp, last_id: i64, events: &[String]) -> anyhow::Result<(Vec<NotifItem>, i64)> {
    let todos = http.get("api/v4/todos", vec![("state", json!("pending")), ("per_page", json!(100))]).await?;
    let mut max_id = last_id;
    let mut items = vec![];
    for todo in arr(&todos) {
        let id = todo["id"].as_i64().unwrap_or(0);
        max_id = max_id.max(id);
        if id <= last_id { continue; }
        let action = s(&todo["action_name"]);
        if !events.contains(&action) { continue; }
        let title = target_title(todo);
        let url = opt(&todo["target_url"]).or_else(|| opt(&todo["target"]["web_url"]));
        items.push(NotifItem {
            source: "gitlab".into(),
            kind: action,
            entity: todo_entity(todo, &title, url.as_deref()),
            title,
            url,
            actor: opt(&todo["author"]["username"]),
            dedup_key: format!("gitlab:{id}"),
            body: opt(&todo["body"]),
            at: None,
        });
    }
    Ok((items, max_id))
}

/// Curseur de départ (premier lancement) : le to-do le plus récent.
pub async fn baseline(http: &ServiceHttp) -> anyhow::Result<i64> {
    let todos = http.get("api/v4/todos", vec![("state", json!("pending")), ("per_page", json!(100))]).await?;
    Ok(arr(&todos).iter().filter_map(|t| t["id"].as_i64()).max().unwrap_or(0))
}

/// L'utilisateur est-il mentionné (`@nom`) ? (déjà couvert par la boîte To-Do)
fn mentions_user(body: &str, me: &str) -> bool {
    let re = regex::Regex::new(&format!(r"(^|[^\w@])@{}(\b|$)", regex::escape(me))).unwrap();
    re.is_match(body)
}

fn mr_title(mr: &Value) -> String {
    let reference = mr["references"]["full"].as_str().map(str::to_string).unwrap_or_else(|| format!("!{}", s(&mr["iid"])));
    format!("{reference} « {} »", s(&mr["title"]))
}

const MAX_MRS: usize = 30;

/// Commentaires de MR hors boîte To-Do : sur une MR de l'utilisateur (`mr_comment`), réponse
/// dans un fil où il a commenté (`reply`). Les mentions sont ignorées (To-Do).
pub async fn fetch_notes(http: &ServiceHttp, since: Option<&str>, events: &[String], now_iso: &str) -> anyhow::Result<(Vec<NotifItem>, String)> {
    let want = |k: &str| events.iter().any(|e| e == k);
    if !want("mr_comment") && !want("reply") { return Ok((vec![], now_iso.into())); }
    let me = s(&http.get("api/v4/user", vec![]).await?["username"]);
    let cutoff = since.and_then(crate::text::parse_ms).unwrap_or_else(|| crate::text::now_ms() - 24 * 3_600_000);
    let updated_after = crate::text::iso(chrono::DateTime::from_timestamp_millis(cutoff).unwrap_or_default());
    let query = |who: &'static str| vec![
        ("scope", json!("all")), ("updated_after", json!(updated_after)), ("per_page", json!(MAX_MRS)), ("order_by", json!("updated_at")), (who, json!(me)),
    ];
    let (authored, reviewed) = tokio::try_join!(http.get("api/v4/merge_requests", query("author_username")), http.get("api/v4/merge_requests", query("reviewer_username")))?;
    let mut mrs: Vec<Value> = vec![];
    for mr in arr(&authored).iter().chain(arr(&reviewed)) {
        let key = (s(&mr["project_id"]), s(&mr["iid"]));
        if let Some(existing) = mrs.iter_mut().find(|m| (s(&m["project_id"]), s(&m["iid"])) == key) { *existing = mr.clone(); } else { mrs.push(mr.clone()); }
    }
    let mut items = vec![];
    for mr in mrs.iter().take(MAX_MRS) {
        let base = format!("api/v4/projects/{}/merge_requests/{}", s(&mr["project_id"]), s(&mr["iid"]));
        let Ok(discussions) = http.get(&format!("{base}/discussions"), vec![("per_page", json!(100))]).await else { continue };
        let mine = mr["author"]["username"].as_str() == Some(me.as_str());
        let entity = EntityRef {
            source: "gitlab".into(),
            r#type: "merge_request".into(),
            key: mr["references"]["full"].as_str().map(str::to_string).unwrap_or_else(|| format!("{}!{}", s(&mr["project_id"]), s(&mr["iid"]))),
            title: mr_title(mr),
            url: opt(&mr["web_url"]),
            participants: Some(mr["author"]["username"].as_str().map(|a| vec![a.to_string()]).unwrap_or_default()),
        };
        for discussion in arr(&discussions) {
            let notes: Vec<&Value> = arr(&discussion["notes"]).iter().filter(|n| n["system"] != true).collect();
            let i_commented = notes.iter().any(|n| n["author"]["username"].as_str() == Some(me.as_str()));
            for note in notes {
                if note["author"]["username"].as_str() == Some(me.as_str()) { continue; }
                if note["created_at"].as_str().and_then(crate::text::parse_ms).is_none_or(|t| t <= cutoff) { continue; }
                if mentions_user(note["body"].as_str().unwrap_or_default(), &me) { continue; }
                let kind = if i_commented { "reply" } else if mine { "mr_comment" } else { continue };
                if !want(kind) { continue; }
                items.push(NotifItem {
                    source: "gitlab".into(),
                    kind: kind.into(),
                    title: mr_title(mr),
                    url: Some(format!("{}#note_{}", s(&mr["web_url"]), s(&note["id"]))),
                    actor: opt(&note["author"]["username"]),
                    dedup_key: format!("gitlab:note:{}", s(&note["id"])),
                    entity: Some(entity.clone()),
                    body: opt(&note["body"]),
                    at: opt(&note["created_at"]),
                });
            }
        }
    }
    Ok((items, now_iso.into()))
}
