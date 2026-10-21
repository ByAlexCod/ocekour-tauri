//! Ingestion Rocket.Chat : TOUT message d'un salon de l'utilisateur, reçu comme envoyé. Un
//! curseur par salon évite les doublons. Premier passage : part de « maintenant ».

use super::entity::ContextEvent;
use super::state::RoomCursors;
use crate::notifications::state::EntityRef;
use crate::services::http::ServiceHttp;
use crate::services::s;
use serde_json::{Value, json};

const MAX_ROOMS: usize = 40;
const MAX_PER_ROOM: usize = 30;

fn history_path(t: &str) -> Option<&'static str> {
    match t { "c" => Some("api/v1/channels.history"), "p" => Some("api/v1/groups.history"), "d" => Some("api/v1/im.history"), _ => None }
}

fn room_path(t: &str) -> &'static str {
    match t { "p" => "group", "d" => "direct", _ => "channel" }
}

/// Dernière activité connue d'un salon (message reçu OU envoyé).
fn activity_at(sub: &Value) -> Option<&str> { sub["lm"].as_str().or(sub["_updatedAt"].as_str()) }

fn room_entity(sub: &Value, http: &ServiceHttp) -> EntityRef {
    let t = s(&sub["t"]);
    let direct = t == "d";
    let name = sub["name"].as_str().or(sub["fname"].as_str()).map(str::to_string).unwrap_or_else(|| s(&sub["rid"]));
    let label = format!("{}{name}", if direct { "@" } else { "#" });
    let title = match sub["fname"].as_str() { Some(f) if !direct => format!("{label} ({f})"), _ => label.clone() };
    let target = if direct { s(&sub["rid"]) } else { s(&sub["name"]) };
    EntityRef {
        source: "rocketchat".into(),
        r#type: if direct { "dm" } else { "channel" }.into(),
        key: label,
        title,
        url: Some(http.web(&format!("{}/{target}", room_path(&t)))),
        participants: None,
    }
}

/// Messages depuis le dernier passage, dans tous les salons ; au premier passage, pose les
/// curseurs à `now_iso` sans rien signaler.
pub async fn fetch(http: &ServiceHttp, cursors: &RoomCursors, now_iso: &str, first_run: bool) -> anyhow::Result<(Vec<ContextEvent>, RoomCursors)> {
    let me = s(&http.get("api/v1/me", vec![]).await?["username"]);
    let subs_v = http.get("api/v1/subscriptions.get", vec![]).await?;
    let subs = subs_v["update"].as_array().cloned().unwrap_or_default();
    let mut next = cursors.clone();
    let mut events = vec![];
    if first_run {
        for sub in &subs { next.insert(s(&sub["rid"]), now_iso.into()); }
        return Ok((events, next));
    }
    let ms = crate::text::parse_ms;
    let mut active: Vec<&Value> = subs
        .iter()
        .filter(|sub| {
            if history_path(sub["t"].as_str().unwrap_or_default()).is_none() { return false; }
            let Some(cursor) = next.get(&s(&sub["rid"])) else { return true };
            sub["unread"].as_i64().unwrap_or(0) > 0 || activity_at(sub).and_then(ms).zip(ms(cursor)).is_some_and(|(a, c)| a > c)
        })
        .collect();
    active.sort_by(|a, b| activity_at(b).unwrap_or_default().cmp(activity_at(a).unwrap_or_default()));
    active.truncate(MAX_ROOMS);

    for sub in active {
        let rid = s(&sub["rid"]);
        let t = s(&sub["t"]);
        let since = next.get(&rid).cloned().or_else(|| sub["ls"].as_str().map(str::to_string)).unwrap_or_else(|| now_iso.into());
        let entity = room_entity(sub, http);
        let Ok(r) = http.get(history_path(&t).unwrap(), vec![("roomId", json!(rid)), ("oldest", json!(since)), ("count", json!(MAX_PER_ROOM)), ("unreads", json!(false))]).await else { continue };
        for m in r["messages"].as_array().map(|a| a.as_slice()).unwrap_or_default() {
            if !m["t"].is_null() { continue; } // message système (arrivée, départ…)
            let Some(text) = m["msg"].as_str().filter(|t| !t.is_empty()) else { continue };
            let at = m["ts"].as_str().unwrap_or(now_iso).to_string();
            if ms(&at).zip(ms(&since)).is_some_and(|(a, c)| a <= c) { continue; }
            let author = m["u"]["username"].as_str().map(str::to_string);
            let mine = author.as_deref() == Some(me.as_str());
            let target = if t == "d" { rid.clone() } else { s(&sub["name"]) };
            events.push(ContextEvent {
                entity: entity.clone(),
                kind: "message".into(),
                at,
                summary: if mine { "Vous avez écrit".into() } else { format!("@{} a écrit", author.as_deref().unwrap_or("?")) },
                actor: author,
                body: Some(text.into()),
                url: Some(http.web(&format!("{}/{target}?msg={}", room_path(&t), s(&m["_id"])))),
                dedup_key: format!("rocketchat:{}", s(&m["_id"])),
            });
        }
        next.insert(rid, now_iso.into());
    }
    Ok((events, next))
}
