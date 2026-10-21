//! Outils Rocket.Chat (API REST v1). Salons désignés par #canal, @utilisateur, id ou URL.

use super::http::ServiceHttp;
use super::markup::{SignatureFormat, sign};
use super::s;
use super::tools::{WriteResult, read, write};
use crate::mcp::{Args, Tool};
use crate::obj;
use crate::schema::{self as z, S};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone)]
struct Room {
    rid: String,
    name: String,
    /// c : canal public, p : privé, d : message direct.
    t: String,
    fname: Option<String>,
}

/// Nom cherché et message direct ou non, à partir de la saisie de l'agent.
pub fn room_query(input: &str) -> (String, bool) {
    let value = input.trim();
    static URL: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"/(channel|group|direct)/([^/?#]+)").unwrap());
    if let Some(c) = URL.captures(value) {
        return (percent_encoding::percent_decode_str(&c[2]).decode_utf8_lossy().to_string(), &c[1] == "direct");
    }
    if let Some(name) = value.strip_prefix('@') { return (name.into(), true); }
    (value.strip_prefix('#').unwrap_or(value).into(), false)
}

fn room_s() -> S { z::string().min(1).describe("#canal, @utilisateur (message direct), nom, id du salon, ou URL web") }
fn arr(v: &Value) -> &[Value] { v.as_array().map(|a| a.as_slice()).unwrap_or_default() }
fn opt(a: &Value, k: &str) -> Value { a.get(k).cloned().unwrap_or(Value::Null) }

async fn rooms(http: &ServiceHttp) -> anyhow::Result<Vec<Room>> {
    let subs = http.get("api/v1/subscriptions.get", vec![]).await?;
    Ok(arr(&subs["update"]).iter().map(|r| Room { rid: s(&r["rid"]), name: s(&r["name"]), t: s(&r["t"]), fname: r["fname"].as_str().map(Into::into) }).collect())
}

async fn find_room(http: &ServiceHttp, input: &str) -> anyhow::Result<Option<Room>> {
    let (name, direct) = room_query(input);
    Ok(rooms(http).await?.into_iter().find(|r| {
        r.rid == name || ((!direct || r.t == "d") && (r.name == name || r.fname.as_deref() == Some(&name) || r.name.to_lowercase() == name.to_lowercase()))
    }))
}

async fn room(http: &ServiceHttp, input: &str) -> anyhow::Result<Room> {
    find_room(http, input).await?.ok_or_else(|| anyhow::anyhow!("Salon « {input} » introuvable parmi ceux de l'utilisateur (rocketchat_canaux pour la liste)."))
}

fn link(http: &ServiceHttp, r: &Room, id: Option<&str>) -> String {
    let kind = match r.t.as_str() { "p" => "group", "d" => "direct", _ => "channel" };
    let name = if r.t == "d" { &r.rid } else { &r.name };
    http.web(&format!("{kind}/{name}{}", id.map(|i| format!("?msg={i}")).unwrap_or_default()))
}

fn message(m: &Value) -> Value {
    let reactions = m["reactions"].as_object().map(|r| r.iter().map(|(emoji, v)| (emoji.clone(), v["usernames"].clone())).collect::<Map<_, _>>());
    obj! {
        "id" => m["_id"], "auteur" => m["u"]["username"], "texte" => m["msg"], "date" => m["ts"],
        "fil" => m["tmid"], "reponses" => m["tcount"],
        "modifie" => if m["editedAt"].is_null() { None } else { Some(true) },
        "reactions" => reactions,
        "piecesJointes" => m["attachments"].as_array().map(|a| a.iter().map(|p| if p["title"].is_null() { p["text"].clone() } else { p["title"].clone() }).collect::<Vec<_>>()),
    }
}

/// Valeur « vraie » au sens JS (0, "", false, null : absents).
fn truthy(v: &Value) -> Option<Value> {
    match v {
        Value::Null | Value::Bool(false) => None,
        Value::Number(n) if n.as_f64() == Some(0.0) => None,
        Value::String(s) if s.is_empty() => None,
        other => Some(other.clone()),
    }
}

pub fn tools() -> Vec<Tool> {
    const R: &str = "rocketchat";
    let mut t = vec![];

    t.push(read(R, "rocketchat_canaux", "Salons de l’utilisateur", "Canaux, groupes privés et messages directs de l'utilisateur, avec les non-lus.",
        vec![("recherche", z::string().optional())],
        |a, http| async move {
            let subs = http.get("api/v1/subscriptions.get", vec![]).await?;
            let search = a.str("recherche").map(|r| r.to_lowercase());
            Ok(Value::Array(arr(&subs["update"]).iter()
                .filter(|r| search.as_ref().is_none_or(|q| format!("{} {}", s(&r["name"]), s(&r["fname"])).to_lowercase().contains(q)))
                .map(|r| {
                    let kind = s(&r["t"]);
                    obj! {
                        "id" => r["rid"],
                        "nom" => if kind == "d" { format!("@{}", s(&r["name"])) } else { format!("#{}", s(&r["name"])) },
                        "titre" => if r["fname"] != r["name"] { r["fname"].clone() } else { Value::Null },
                        "type" => match kind.as_str() { "c" => "public".to_string(), "p" => "privé".into(), "d" => "direct".into(), other => other.into() },
                        "nonLus" => truthy(&r["unread"]),
                        "mention" => truthy(&r["userMentions"]),
                    }
                }).collect()))
        }));

    t.push(read(R, "rocketchat_historique", "Historique d’un salon ou d’un fil", "Messages les plus récents d’abord ; « fil » : réponses d’un fil (id du message parent).",
        vec![
            ("salon", room_s()),
            ("fil", z::string().optional().describe("Id du message parent du fil")),
            ("nombre", z::int().min(1).max(500).optional().describe("50 par défaut")),
            ("avant", z::string().optional().describe("Messages antérieurs à cette date (ISO 8601)")),
            ("apres", z::string().optional().describe("Messages postérieurs à cette date (ISO 8601)")),
            ("decalage", z::int().min(0).optional()),
        ],
        |a, http| async move {
            let r = room(&http, &a.string("salon")).await?;
            let count = json!(a.int("nombre").unwrap_or(50));
            let h = if let Some(fil) = a.str("fil") {
                http.get("api/v1/chat.getThreadMessages", vec![("tmid", json!(fil)), ("count", count), ("offset", opt(&a, "decalage"))]).await?
            } else {
                let endpoint = match r.t.as_str() { "p" => "groups.history", "d" => "im.history", _ => "channels.history" };
                http.get(&format!("api/v1/{endpoint}"), vec![
                    ("roomId", json!(r.rid)), ("count", count), ("latest", opt(&a, "avant")), ("oldest", opt(&a, "apres")), ("offset", opt(&a, "decalage")),
                ]).await?
            };
            Ok(json!({ "salon": link(&http, &r, None), "messages": arr(&h["messages"]).iter().map(message).collect::<Vec<_>>() }))
        }));

    t.push(read(R, "rocketchat_chercher", "Chercher des messages", "Recherche de messages dans un salon.",
        vec![
            ("salon", room_s()),
            ("recherche", z::string().min(1)),
            ("nombre", z::int().min(1).max(200).optional()),
            ("decalage", z::int().min(0).optional()),
        ],
        |a, http| async move {
            let r = room(&http, &a.string("salon")).await?;
            let found = http.get("api/v1/chat.search", vec![
                ("roomId", json!(r.rid)), ("searchText", json!(a.string("recherche"))), ("count", json!(a.int("nombre").unwrap_or(50))), ("offset", opt(&a, "decalage")),
            ]).await?;
            Ok(Value::Array(arr(&found["messages"]).iter().map(|m| {
                let mut v = message(m);
                v["url"] = json!(link(&http, &r, m["_id"].as_str()));
                v
            }).collect()))
        }));

    t.push(write(R, "rocketchat_envoyer", "Envoyer un message",
        "Message dans un canal, un fil (« fil » : id du message parent) ou en direct (@utilisateur). Markdown Rocket.Chat.",
        vec![("salon", room_s()), ("texte", z::string().min(1)), ("fil", z::string().optional())],
        |a| Ok(format!("{}{}", a.string("salon"), a.str("fil").map(|f| format!(" (fil {f})")).unwrap_or_default())),
        false,
        |a, http| async move {
            let salon = a.string("salon");
            let found = find_room(&http, &salon).await?;
            let (name, direct) = room_query(&salon);
            if found.is_none() && !direct { anyhow::bail!("Salon « {salon} » introuvable parmi ceux de l'utilisateur."); }
            let mut body = match &found {
                Some(r) => json!({ "roomId": r.rid }),
                None => json!({ "channel": format!("@{name}") }),
            };
            body["text"] = json!(sign(&a.string("texte"), SignatureFormat::Markdown));
            if let Some(fil) = a.str("fil") { body["tmid"] = json!(fil); }
            let r = http.post("api/v1/chat.postMessage", body).await?;
            let m = &r["message"];
            let target = found.unwrap_or(Room { rid: s(&m["rid"]), name, t: "d".into(), fname: None });
            let url = link(&http, &target, m["_id"].as_str());
            Ok(WriteResult { value: json!({ "message": m["_id"], "url": url }), url: Some(url) })
        }));

    t.push(write(R, "rocketchat_reagir", "Réagir à un message", "Ajoute (ou retire) une réaction emoji, ex. « :thumbsup: ».",
        vec![
            ("message", z::string().min(1).describe("Id du message")),
            ("emoji", z::string().min(1)),
            ("retirer", z::boolean().optional()),
        ],
        |a| Ok(format!("message {} {}", a.string("message"), a.string("emoji"))),
        false,
        |a, http| async move {
            let raw = a.string("emoji");
            let emoji = if raw.starts_with(':') { raw } else { format!(":{raw}:") };
            let retirer = a.bool("retirer").unwrap_or(false);
            let mut body = json!({ "messageId": a.string("message"), "emoji": emoji });
            if retirer { body["shouldReact"] = json!(false); }
            http.post("api/v1/chat.react", body).await?;
            Ok(WriteResult { value: json!({ "message": a.string("message"), "emoji": emoji, "retire": retirer }), url: None })
        }));

    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_queries() {
        assert_eq!(room_query("#general"), ("general".into(), false));
        assert_eq!(room_query("@alice"), ("alice".into(), true));
        assert_eq!(room_query("https://chat.example/direct/abc%20d?msg=1"), ("abc d".into(), true));
        assert_eq!(room_query("https://chat.example/group/equipe"), ("equipe".into(), false));
        assert_eq!(room_query(" equipe "), ("equipe".into(), false));
    }
}
