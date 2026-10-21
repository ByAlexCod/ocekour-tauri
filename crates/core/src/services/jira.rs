//! Outils Jira Data Center (API REST v2 et Agile 1.0). Tickets désignés par clé ou URL.

use super::http::ServiceHttp;
use super::markup::{SignatureFormat, markdown_to_jira, sign};
use super::s;
use super::tools::{WriteResult, read, write};
use crate::mcp::{Args, Tool};
use crate::obj;
use crate::schema::{self as z, S};
use serde_json::{Map, Value, json};

/// Clé de ticket (`PAY-123`) à partir d'une clé ou d'une URL web Jira.
pub fn issue_key(input: &str) -> anyhow::Result<String> {
    static KEY: once_cell::sync::Lazy<fancy_regex::Regex> =
        once_cell::sync::Lazy::new(|| fancy_regex::Regex::new(r"(?i)([A-Z][A-Z0-9_]+-\d+)(?![\w-])").unwrap());
    KEY.captures(input.trim())
        .ok()
        .flatten()
        .map(|c| c[1].to_uppercase())
        .ok_or_else(|| anyhow::anyhow!("Clé de ticket Jira illisible : {input} (ex. PAY-123)."))
}

/// Date au format attendu par Jira : `2026-09-25T09:00:00.000+0200` (heure locale).
pub fn jira_date(date: chrono::DateTime<chrono::Local>) -> String {
    date.format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string()
}

fn key_s() -> S { z::string().min(1).describe("Clé du ticket (PAY-123) ou URL web") }
fn format_s() -> S {
    z::enumeration(&["markdown", "wiki"]).optional().describe("markdown (défaut, converti en wiki-markup Jira) ou wiki (texte Jira tel quel)")
}

/// Texte écrit par l'agent → wiki-markup Jira signé.
fn text(value: &str, format: Option<String>) -> String {
    sign(&if format.as_deref() == Some("wiki") { value.to_string() } else { markdown_to_jira(value) }, SignatureFormat::Wiki)
}

fn person(u: &Value) -> Value {
    if !u.is_object() { return Value::Null; }
    let id = if u["name"].is_null() { &u["key"] } else { &u["name"] };
    json!(format!("{} ({})", s(&u["displayName"]), s(id)))
}

/// Valeur de champ lisible (options, utilisateurs, listes…).
fn simple(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Array(a) => Value::Array(a.iter().map(simple).collect()),
        Value::Object(v) => {
            if v.contains_key("displayName") { return person(value); }
            if v.contains_key("value") {
                return if v.get("child").is_some_and(|c| !c.is_null()) { json!(format!("{} / {}", s(&v["value"]), s(&v["child"]["value"]))) } else { v["value"].clone() };
            }
            if v.contains_key("name") { return v["name"].clone(); }
            if v.contains_key("key") { return v["key"].clone(); }
            value.clone()
        }
        other => other.clone(),
    }
}

const DEFAULT_FIELDS: [&str; 9] = ["summary", "status", "issuetype", "priority", "assignee", "reporter", "updated", "created", "labels"];

fn browse(http: &ServiceHttp, key: &str) -> String { http.web(&format!("browse/{key}")) }

fn summary(http: &ServiceHttp, i: &Value) -> Value {
    let f = &i["fields"];
    obj! {
        "cle" => i["key"], "resume" => f["summary"], "statut" => f["status"]["name"], "type" => f["issuetype"]["name"],
        "priorite" => f["priority"]["name"], "assigne" => person(&f["assignee"]), "rapporteur" => person(&f["reporter"]),
        "libelles" => f["labels"], "cree" => f["created"], "maj" => f["updated"], "url" => browse(http, &s(&i["key"])),
    }
}

async fn me(http: &ServiceHttp) -> anyhow::Result<String> { Ok(s(&http.get("rest/api/2/myself", vec![]).await?["name"])) }

fn arr(v: &Value) -> &[Value] { v.as_array().map(|a| a.as_slice()).unwrap_or_default() }
fn opt(a: &Value, k: &str) -> Value { a.get(k).cloned().unwrap_or(Value::Null) }

pub fn tools() -> Vec<Tool> {
    const J: &str = "jira";
    let mut t = vec![];

    t.push(read(J, "jira_chercher", "Chercher des tickets (JQL)",
        "Recherche JQL, ex. « project = PAY AND status = \"In Progress\" AND assignee = currentUser() ORDER BY updated DESC ».",
        vec![
            ("jql", z::string().min(1)),
            ("champs", z::array(z::string()).optional().describe("Champs renvoyés en plus du résumé habituel")),
            ("debut", z::int().min(0).optional().describe("Index du premier résultat (pagination)")),
            ("max", z::int().min(1).max(200).optional().describe("50 par défaut")),
        ],
        |a, http| async move {
            let extra: Vec<String> = a["champs"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_string)).collect();
            let fields: Vec<String> = DEFAULT_FIELDS.iter().map(|f| f.to_string()).chain(extra.clone()).collect();
            let r = http.post("rest/api/2/search", json!({ "jql": a["jql"], "startAt": a.int("debut").unwrap_or(0), "maxResults": a.int("max").unwrap_or(50), "fields": fields })).await?;
            let tickets: Vec<Value> = arr(&r["issues"]).iter().map(|i| {
                let mut v = summary(&http, i);
                for c in &extra {
                    let x = simple(&i["fields"][c]);
                    if !x.is_null() { v.as_object_mut().unwrap().insert(c.clone(), x); }
                }
                v
            }).collect();
            Ok(obj! { "total" => r["total"], "debut" => r["startAt"], "tickets" => tickets })
        }));

    t.push(read(J, "jira_ticket_lire", "Lire un ticket",
        "Ticket complet : description, champs (personnalisés nommés), commentaires, liens, sous-tâches, pièces jointes.",
        vec![("ticket", key_s())],
        |a, http| async move {
            let key = issue_key(&a.string("ticket"))?;
            let i = http.get(&format!("rest/api/2/issue/{key}"), vec![("expand", json!("names"))]).await?;
            let f = &i["fields"];
            let names = &i["names"];
            let mut custom = Map::new();
            if let Some(fields) = f.as_object() {
                for (k, v) in fields {
                    if !k.starts_with("customfield_") || v.is_null() { continue; }
                    let x = simple(v);
                    if x.is_null() || x.as_array().is_some_and(|a| a.is_empty()) { continue; }
                    let name = names[k].as_str().unwrap_or(k);
                    custom.insert(format!("{name} ({k})"), x);
                }
            }
            let mut out = summary(&http, &i);
            let extra = obj! {
                "description" => f["description"], "resolution" => f["resolution"]["name"],
                "composants" => simple(&f["components"]), "versions" => simple(&f["fixVersions"]), "parent" => f["parent"]["key"],
                "sousTaches" => arr(&f["subtasks"]).iter().map(|st| obj! { "cle" => st["key"], "resume" => st["fields"]["summary"], "statut" => st["fields"]["status"]["name"] }).collect::<Vec<_>>(),
                "liens" => arr(&f["issuelinks"]).iter().map(|l| {
                    let outward = l["outwardIssue"].is_object();
                    let other = if outward { &l["outwardIssue"] } else { &l["inwardIssue"] };
                    obj! { "type" => if outward { l["type"]["outward"].clone() } else { l["type"]["inward"].clone() }, "cle" => other["key"], "resume" => other["fields"]["summary"] }
                }).collect::<Vec<_>>(),
                "piecesJointes" => arr(&f["attachment"]).iter().map(|p| obj! { "nom" => p["filename"], "taille" => p["size"], "url" => p["content"] }).collect::<Vec<_>>(),
                "tempsPasse" => f["timetracking"]["timeSpent"],
                "champs" => Value::Object(custom),
                "commentaires" => arr(&f["comment"]["comments"]).iter().map(|c| obj! { "id" => c["id"], "auteur" => person(&c["author"]), "date" => c["created"], "texte" => c["body"] }).collect::<Vec<_>>(),
            };
            out.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            Ok(out)
        }));

    t.push(read(J, "jira_projets", "Projets, types et champs",
        "Sans paramètre : projets. Avec projet : types de ticket. Avec projet et type : champs de création (obligatoires, valeurs possibles).",
        vec![("projet", z::string().optional().describe("Clé du projet (PAY)")), ("type", z::string().optional().describe("Nom ou id du type de ticket"))],
        |a, http| async move {
            let Some(projet) = a.str("projet") else {
                let list = http.get("rest/api/2/project", vec![]).await?;
                return Ok(Value::Array(arr(&list).iter().map(|p| obj! { "cle" => p["key"], "nom" => p["name"], "type" => p["projectTypeKey"] }).collect()));
            };
            let project = crate::rancher::enc(&projet.to_uppercase());
            let types = http.get(&format!("rest/api/2/issue/createmeta/{project}/issuetypes"), vec![("maxResults", json!(100))]).await?["values"].clone();
            let Some(wanted) = a.str("type") else {
                return Ok(Value::Array(arr(&types).iter().map(|ty| obj! { "id" => ty["id"], "nom" => ty["name"], "sousTache" => ty["subtask"] }).collect()));
            };
            let found = arr(&types).iter().find(|ty| ty["id"] == json!(wanted) || s(&ty["name"]).to_lowercase() == wanted.to_lowercase());
            let Some(ty) = found else { anyhow::bail!("Type « {wanted} » absent du projet {projet}.") };
            let fields = http.get(&format!("rest/api/2/issue/createmeta/{project}/issuetypes/{}", s(&ty["id"])), vec![("maxResults", json!(200))]).await?["values"].clone();
            Ok(Value::Array(arr(&fields).iter().map(|c| obj! {
                "id" => c["fieldId"], "nom" => c["name"], "obligatoire" => c["required"], "type" => c["schema"]["type"],
                "valeurs" => c["allowedValues"].as_array().map(|v| v.iter().take(50).map(|x| {
                    for k in ["value", "name", "key", "id"] { if !x[k].is_null() { return x[k].clone(); } }
                    Value::Null
                }).collect::<Vec<_>>()),
            }).collect()))
        }));

    t.push(read(J, "jira_transitions", "Transitions possibles", "Changements de statut possibles pour un ticket (pour jira_transition).",
        vec![("ticket", key_s())],
        |a, http| async move {
            let r = http.get(&format!("rest/api/2/issue/{}/transitions", issue_key(&a.string("ticket"))?), vec![]).await?;
            Ok(Value::Array(arr(&r["transitions"]).iter().map(|tr| obj! { "id" => tr["id"], "nom" => tr["name"], "versStatut" => tr["to"]["name"] }).collect()))
        }));

    t.push(read(J, "jira_utilisateurs", "Chercher un utilisateur",
        "Utilisateurs Jira par nom, identifiant ou e-mail (nom d'utilisateur pour jira_assigner).",
        vec![("recherche", z::string().min(1))],
        |a, http| async move {
            let list = http.get("rest/api/2/user/search", vec![("username", a["recherche"].clone()), ("maxResults", json!(20))]).await?;
            Ok(Value::Array(arr(&list).iter().map(|u| obj! { "nom" => u["displayName"], "identifiant" => u["name"], "email" => u["emailAddress"], "actif" => u["active"] }).collect()))
        }));

    t.push(read(J, "jira_tableaux", "Tableaux agiles", "Tableaux Scrum / Kanban, filtrables par projet ou nom.",
        vec![("projet", z::string().optional()), ("nom", z::string().optional()), ("debut", z::int().min(0).optional())],
        |a, http| async move {
            let r = http.get("rest/agile/1.0/board", vec![("projectKeyOrId", opt(&a, "projet")), ("name", opt(&a, "nom")), ("startAt", opt(&a, "debut")), ("maxResults", json!(50))]).await?;
            Ok(Value::Array(arr(&r["values"]).iter().map(|b| obj! { "id" => b["id"], "nom" => b["name"], "type" => b["type"] }).collect()))
        }));

    t.push(read(J, "jira_sprints", "Sprints d’un tableau", "Sprints d’un tableau (actifs par défaut).",
        vec![
            ("tableau", z::int().min(1)),
            ("etat", z::enumeration(&["active", "future", "closed", "active,future"]).optional()),
            ("debut", z::int().min(0).optional()),
        ],
        |a, http| async move {
            let r = http.get(&format!("rest/agile/1.0/board/{}/sprint", s(&a["tableau"])), vec![
                ("state", json!(a.str("etat").unwrap_or_else(|| "active".into()))), ("startAt", opt(&a, "debut")), ("maxResults", json!(50)),
            ]).await?;
            Ok(Value::Array(arr(&r["values"]).iter().map(|sp| obj! {
                "id" => sp["id"], "nom" => sp["name"], "etat" => sp["state"], "debut" => sp["startDate"], "fin" => sp["endDate"], "objectif" => sp["goal"],
            }).collect()))
        }));

    t.push(read(J, "jira_sprint_tickets", "Tickets d’un sprint", "Tickets d’un sprint, avec filtre JQL facultatif.",
        vec![("sprint", z::int().min(1)), ("jql", z::string().optional()), ("debut", z::int().min(0).optional()), ("max", z::int().min(1).max(200).optional())],
        |a, http| async move {
            let r = http.get(&format!("rest/agile/1.0/sprint/{}/issue", s(&a["sprint"])), vec![
                ("jql", opt(&a, "jql")), ("startAt", opt(&a, "debut")), ("maxResults", json!(a.int("max").unwrap_or(50))), ("fields", json!(DEFAULT_FIELDS.join(","))),
            ]).await?;
            Ok(obj! { "total" => r["total"], "tickets" => arr(&r["issues"]).iter().map(|i| summary(&http, i)).collect::<Vec<_>>() })
        }));

    // --- Écriture -------------------------------------------------------------------------

    t.push(write(J, "jira_ticket_creer", "Créer un ticket",
        "Crée un ticket. Champs supplémentaires (personnalisés…) au format de l’API Jira dans « champs » ; voir jira_projets.",
        vec![
            ("projet", z::string().min(1).describe("Clé du projet")),
            ("type", z::string().min(1).describe("Type de ticket (nom), ex. Bug, Story, Tâche")),
            ("resume", z::string().min(1)),
            ("description", z::string().optional()),
            ("format", format_s()),
            ("priorite", z::string().optional()),
            ("libelles", z::array(z::string()).optional()),
            ("assigne", z::string().optional().describe("Nom d'utilisateur ; « moi » pour soi-même")),
            ("parent", z::string().optional().describe("Clé du parent (sous-tâche)")),
            ("champs", z::record(z::any()).optional().describe("Autres champs, ex. { \"customfield_10010\": \"PAY-1\" }")),
        ],
        |a| Ok(format!("{} : {}", a.string("projet").to_uppercase(), a.string("resume").chars().take(80).collect::<String>())),
        false,
        |a, http| async move {
            let assignee = match a.str("assigne").as_deref() { Some("moi") => Some(me(&http).await?), other => other.map(str::to_string) };
            let mut fields = obj! {
                "project" => json!({ "key": a.string("projet").to_uppercase() }), "issuetype" => json!({ "name": a["type"] }),
                "summary" => a["resume"], "description" => text(&a.string("description"), a.str("format")),
            };
            let f = fields.as_object_mut().unwrap();
            if let Some(p) = a.str("priorite") { f.insert("priority".into(), json!({ "name": p })); }
            if a["libelles"].is_array() { f.insert("labels".into(), a["libelles"].clone()); }
            if let Some(u) = assignee { f.insert("assignee".into(), json!({ "name": u })); }
            if let Some(p) = a.str("parent") { f.insert("parent".into(), json!({ "key": issue_key(&p)? })); }
            if let Some(extra) = a["champs"].as_object() { f.extend(extra.clone()); }
            let r = http.post("rest/api/2/issue", json!({ "fields": fields })).await?;
            let url = browse(&http, &s(&r["key"]));
            Ok(WriteResult { value: json!({ "cle": r["key"], "url": url }), url: Some(url) })
        }));

    t.push(write(J, "jira_ticket_modifier", "Modifier un ticket",
        "Modifie des champs : résumé, description, priorité, libellés, et autres champs au format API.",
        vec![
            ("ticket", key_s()),
            ("resume", z::string().optional()),
            ("description", z::string().optional()),
            ("format", format_s()),
            ("priorite", z::string().optional()),
            ("libelles", z::array(z::string()).optional().describe("Remplace les libellés")),
            ("champs", z::record(z::any()).optional()),
        ],
        |a| issue_key(&a.string("ticket")),
        false,
        |a, http| async move {
            let key = issue_key(&a.string("ticket"))?;
            let mut fields = Map::new();
            if let Some(r) = a.str("resume").filter(|r| !r.is_empty()) { fields.insert("summary".into(), json!(r)); }
            if let Some(d) = a.str("description") { fields.insert("description".into(), json!(text(&d, a.str("format")))); }
            if let Some(p) = a.str("priorite").filter(|p| !p.is_empty()) { fields.insert("priority".into(), json!({ "name": p })); }
            if a["libelles"].is_array() { fields.insert("labels".into(), a["libelles"].clone()); }
            if let Some(extra) = a["champs"].as_object() { fields.extend(extra.clone()); }
            if fields.is_empty() { anyhow::bail!("Aucun champ à modifier."); }
            let names: Vec<String> = fields.keys().cloned().collect();
            http.put(&format!("rest/api/2/issue/{key}"), Some(json!({ "fields": fields })), vec![]).await?;
            let url = browse(&http, &key);
            Ok(WriteResult { value: json!({ "cle": key, "modifies": names, "url": url }), url: Some(url) })
        }));

    t.push(write(J, "jira_commenter", "Commenter un ticket", "Ajoute un commentaire au ticket.",
        vec![("ticket", key_s()), ("texte", z::string().min(1)), ("format", format_s())],
        |a| issue_key(&a.string("ticket")),
        false,
        |a, http| async move {
            let key = issue_key(&a.string("ticket"))?;
            let c = http.post(&format!("rest/api/2/issue/{key}/comment"), json!({ "body": text(&a.string("texte"), a.str("format")) })).await?;
            let id = s(&c["id"]);
            let url = format!("{}?focusedCommentId={id}#comment-{id}", browse(&http, &key));
            Ok(WriteResult { value: json!({ "commentaire": c["id"], "url": url }), url: Some(url) })
        }));

    t.push(write(J, "jira_transition", "Changer le statut",
        "Applique une transition (nom ou id : jira_transitions), avec un commentaire facultatif.",
        vec![
            ("ticket", key_s()),
            ("transition", z::string().min(1).describe("Nom (ex. « En cours ») ou id")),
            ("commentaire", z::string().optional()),
            ("format", format_s()),
            ("champs", z::record(z::any()).optional().describe("Champs demandés par la transition (ex. resolution)")),
        ],
        |a| Ok(format!("{} → {}", issue_key(&a.string("ticket"))?, a.string("transition"))),
        false,
        |a, http| async move {
            let key = issue_key(&a.string("ticket"))?;
            let list = http.get(&format!("rest/api/2/issue/{key}/transitions"), vec![]).await?["transitions"].clone();
            let wanted = a.string("transition").to_lowercase();
            let found = arr(&list).iter().find(|x| x["id"] == a["transition"] || s(&x["name"]).to_lowercase() == wanted || s(&x["to"]["name"]).to_lowercase() == wanted).cloned();
            let Some(tr) = found else {
                anyhow::bail!("Transition « {} » impossible pour {key}. Possibles : {}.", a.string("transition"), arr(&list).iter().map(|x| s(&x["name"])).collect::<Vec<_>>().join(", "));
            };
            let mut body = json!({ "transition": { "id": tr["id"] } });
            if a["champs"].is_object() { body["fields"] = a["champs"].clone(); }
            if let Some(c) = a.str("commentaire").filter(|c| !c.is_empty()) {
                body["update"] = json!({ "comment": [{ "add": { "body": text(&c, a.str("format")) } }] });
            }
            http.post(&format!("rest/api/2/issue/{key}/transitions"), body).await?;
            let url = browse(&http, &key);
            let status = if tr["to"]["name"].is_null() { tr["name"].clone() } else { tr["to"]["name"].clone() };
            Ok(WriteResult { value: json!({ "cle": key, "statut": status, "url": url }), url: Some(url) })
        }));

    t.push(write(J, "jira_assigner", "Assigner un ticket",
        "Assigne le ticket (nom d'utilisateur, « moi », ou « personne » pour désassigner).",
        vec![("ticket", key_s()), ("utilisateur", z::string().min(1))],
        |a| Ok(format!("{} → {}", issue_key(&a.string("ticket"))?, a.string("utilisateur"))),
        false,
        |a, http| async move {
            let key = issue_key(&a.string("ticket"))?;
            let name = match a.string("utilisateur").as_str() {
                "personne" => Value::Null,
                "moi" => json!(me(&http).await?),
                other => json!(other),
            };
            http.put(&format!("rest/api/2/issue/{key}/assignee"), Some(json!({ "name": name })), vec![]).await?;
            let url = browse(&http, &key);
            Ok(WriteResult { value: json!({ "cle": key, "assigne": name, "url": url }), url: Some(url) })
        }));

    t.push(write(J, "jira_temps_saisir", "Saisir du temps passé", "Ajoute une saisie de temps (worklog) au ticket, ex. « 1h 30m ».",
        vec![
            ("ticket", key_s()),
            ("temps", z::string().min(1).describe("Durée Jira : « 2h », « 1h 30m », « 1d »")),
            ("debut", z::string().optional().describe("Début (ISO 8601) ; maintenant par défaut")),
            ("commentaire", z::string().optional()),
            ("format", format_s()),
        ],
        |a| Ok(format!("{} {}", issue_key(&a.string("ticket"))?, a.string("temps"))),
        false,
        |a, http| async move {
            let key = issue_key(&a.string("ticket"))?;
            let started = match a.str("debut") {
                Some(d) => crate::logs::parser::to_date_str(&d)
                    .map(|d| d.with_timezone(&chrono::Local))
                    .ok_or_else(|| anyhow::anyhow!("Date de début illisible (ISO 8601 attendu)."))?,
                None => chrono::Local::now(),
            };
            let w = http.post(&format!("rest/api/2/issue/{key}/worklog"), json!({
                "timeSpent": a["temps"], "started": jira_date(started), "comment": text(&a.string("commentaire"), a.str("format")),
            })).await?;
            let url = format!("{}?focusedWorklogId={}", browse(&http, &key), s(&w["id"]));
            Ok(WriteResult { value: json!({ "saisie": w["id"], "temps": w["timeSpent"], "url": url }), url: Some(url) })
        }));

    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cles_et_dates() {
        assert_eq!(issue_key("pay-12").unwrap(), "PAY-12");
        assert_eq!(issue_key("https://jira.intra/browse/PAY-123?focusedCommentId=1").unwrap(), "PAY-123");
        assert!(issue_key("rien").unwrap_err().to_string().contains("illisible"));
        let d = chrono::DateTime::parse_from_rfc3339("2026-09-25T07:00:00Z").unwrap().with_timezone(&chrono::Local);
        assert!(regex::Regex::new(r"^2026-09-2\dT\d\d:00:00\.000[+-]\d{4}$").unwrap().is_match(&jira_date(d)), "{}", jira_date(d));
    }
}
