//! Faux GitLab, Jira, Confluence et Rocket.Chat (réponses réduites mais au format réel), sous
//! des chemins différents d'un même serveur : /gitlab, /jira, /confluence, /chat.

use super::http::{Req, Resp, TestServer, handler, start};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

pub const GITLAB_TOKEN: &str = "glpat-test-123";
pub const JIRA_TOKEN: &str = "jira-pat-456";
pub const CONFLUENCE_TOKEN: &str = "conf-pat-789";
pub const ROCKETCHAT_TOKEN: &str = "rc-token";
pub const ROCKETCHAT_USER: &str = "rc-user";

/// Diff d'un fichier de la MR !12 : ligne 2 remplacée, une ligne ajoutée.
pub const FAKE_DIFF: &str = "@@ -1,4 +1,5 @@\n un\n-deux\n+deux bis\n+nouvelle\n trois\n quatre\n";

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    /// `/service/chemin brut` (le %2F des projets GitLab est gardé).
    pub path: String,
    pub body: Value,
    pub headers: HashMap<String, String>,
}

pub struct FakeServices {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    _server: TestServer,
}

impl FakeServices {
    pub fn clear(&self) { self.requests.lock().clear(); }
    pub fn last_write(&self) -> Option<Recorded> { self.requests.lock().iter().rev().find(|r| r.method != "GET").cloned() }
}

type Reply = Option<Resp>;

fn json_reply(v: Value) -> Reply { Some(Resp::Json(200, v)) }

fn gitlab(method: &str, path: &str, body: &Value, base: &str) -> Reply {
    let project = json!({ "id": 42, "path_with_namespace": "grp/app", "name": "app", "default_branch": "main", "web_url": format!("{base}/gitlab/grp/app") });
    let p = "/api/v4/projects/grp%2Fapp";
    let mr = json!({
        "iid": 12, "title": "Paiement", "state": "opened", "source_branch": "feat", "target_branch": "main",
        "author": { "username": "amartin" }, "web_url": format!("{base}/gitlab/grp/app/-/merge_requests/12"),
        "diff_refs": { "base_sha": "b", "head_sha": "h", "start_sha": "s" },
    });
    let changes = json!([{ "old_path": "src/a.ts", "new_path": "src/a.ts", "diff": FAKE_DIFF }]);
    match (method, path) {
        ("GET", "/api/v4/user") => json_reply(json!({ "name": "Alex Martin", "username": "amartin" })),
        ("GET", "/api/v4/personal_access_tokens/self") => json_reply(json!({ "scopes": ["read_api"], "expires_at": null })),
        ("GET", "/api/v4/projects") => json_reply(json!([project])),
        ("GET", x) if x == p => json_reply(project),
        ("GET", x) if x == format!("{p}/merge_requests/12") => json_reply(mr),
        ("GET", x) if x == format!("{p}/merge_requests/12/diffs") => Some(Resp::Json(404, json!({ "message": "404 Not Found" }))),
        ("GET", x) if x == format!("{p}/merge_requests/12/changes") => json_reply(json!({ "changes": changes })),
        ("GET", x) if x == format!("{p}/merge_requests/12/versions") => {
            json_reply(json!([{ "id": 7, "head_commit_sha": "h", "base_commit_sha": "b", "start_commit_sha": "s" }]))
        }
        ("GET", x) if x == format!("{p}/merge_requests/12/versions/7") => json_reply(json!({ "diffs": changes })),
        ("POST", x) if x == format!("{p}/merge_requests/12/notes") => Some(Resp::Json(201, json!({ "id": 100, "body": body["body"] }))),
        ("POST", x) if x == format!("{p}/merge_requests/12/discussions") => {
            Some(Resp::Json(201, json!({ "id": "d1", "notes": [{ "id": 101, "body": body["body"], "position": body["position"] }] })))
        }
        ("GET", x) if x == format!("{p}/jobs/5/trace") => {
            Some(Resp::Text(200, "section_start:1:build\r\u{1b}[0K\u{1b}[32;1mOK\u{1b}[0;m\nligne 2\nERREUR finale\n".into()))
        }
        _ => None,
    }
}

fn jira(method: &str, path: &str, body: &Value) -> Reply {
    match (method, path) {
        ("GET", "/rest/api/2/myself") => json_reply(json!({ "displayName": "Alex Martin", "name": "amartin" })),
        ("POST", "/rest/api/2/search") => json_reply(json!({
            "total": 1, "startAt": 0,
            "issues": [{ "key": "PAY-1", "fields": { "summary": "Paiement en double", "status": { "name": "Ouvert" } } }],
        })),
        ("GET", "/rest/api/2/issue/PAY-1") => json_reply(json!({
            "key": "PAY-1",
            "names": { "customfield_10010": "Criticité" },
            "fields": {
                "summary": "Paiement en double", "status": { "name": "Ouvert" }, "description": "h1. Contexte",
                "customfield_10010": { "value": "Haute" }, "customfield_10011": null,
                "comment": { "comments": [{ "id": "1", "author": { "displayName": "Bob", "name": "bob" }, "body": "Vu", "created": "2026-09-01" }] },
            },
        })),
        ("POST", "/rest/api/2/issue/PAY-1/comment") => Some(Resp::Json(201, json!({ "id": "555", "body": body["body"] }))),
        ("GET", "/rest/api/2/issue/PAY-1/transitions") => {
            json_reply(json!({ "transitions": [{ "id": "31", "name": "Démarrer", "to": { "name": "En cours" } }] }))
        }
        ("POST", "/rest/api/2/issue/PAY-1/transitions") => Some(Resp::Empty(204, vec![])),
        _ => None,
    }
}

fn confluence(method: &str, path: &str, body: &Value, authorized: bool, version: &AtomicI64) -> Reply {
    if (method, path) == ("GET", "/rest/api/user/current") {
        return json_reply(if authorized {
            json!({ "type": "known", "username": "amartin", "displayName": "Alex Martin" })
        } else {
            json!({ "type": "anonymous", "displayName": "Anonyme" })
        });
    }
    let page = |title: Value| json!({
        "id": "123", "type": "page", "title": title, "space": { "key": "DEV" },
        "version": { "number": version.load(Ordering::SeqCst), "when": "2026-09-20T10:00:00Z", "by": { "displayName": "Bob" } },
        "body": { "storage": { "value": "<h1>Déployer</h1><p>Lancer :</p><ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">bash</ac:parameter><ac:plain-text-body><![CDATA[make deploy ENV=prod]]></ac:plain-text-body></ac:structured-macro>" } },
        "_links": { "webui": "/display/DEV/D%C3%A9ploiement" },
    });
    match (method, path) {
        ("GET", "/rest/api/content/123") => json_reply(page(json!("Déploiement"))),
        ("PUT", "/rest/api/content/123") => {
            version.store(body["version"]["number"].as_i64().unwrap_or(0), Ordering::SeqCst);
            json_reply(page(body["title"].clone()))
        }
        ("POST", "/rest/api/content/123/label") => json_reply(json!({ "results": [] })),
        _ => None,
    }
}

fn rocketchat(method: &str, path: &str, body: &Value) -> Reply {
    match (method, path) {
        ("GET", "/api/v1/me") => json_reply(json!({ "name": "Alex Martin", "username": "amartin" })),
        ("GET", "/api/v1/subscriptions.get") => json_reply(json!({
            "update": [
                { "rid": "R1", "name": "general", "fname": "general", "t": "c", "unread": 2 },
                { "rid": "D1", "name": "bob", "t": "d" },
            ],
        })),
        ("GET", "/api/v1/channels.history") => json_reply(json!({
            "messages": [{ "_id": "M1", "msg": "Bonjour", "u": { "username": "bob" }, "ts": "2026-09-25T08:00:00Z" }],
        })),
        ("POST", "/api/v1/chat.postMessage") => json_reply(json!({
            "success": true,
            "message": { "_id": "M9", "rid": if body["roomId"].is_null() { json!("D1") } else { body["roomId"].clone() }, "msg": body["text"] },
        })),
        _ => None,
    }
}

pub async fn start_fake_services() -> FakeServices {
    let requests: Arc<Mutex<Vec<Recorded>>> = Arc::default();
    let base: Arc<Mutex<String>> = Arc::default();
    let version = Arc::new(AtomicI64::new(4));
    let h = {
        let (requests, base) = (requests.clone(), base.clone());
        handler(move |r: Req| {
            let (requests, base, version) = (requests.clone(), base.lock().clone(), version.clone());
            async move {
                let body: Value = if r.body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&r.body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&r.body).into()))
                };
                let mut parts = r.raw_path.splitn(3, '/').skip(1);
                let service = parts.next().unwrap_or_default().to_string();
                let path = format!("/{}", parts.next().unwrap_or_default());
                let method = r.method.clone();
                requests.lock().push(Recorded { method: method.clone(), path: format!("/{service}{path}"), body: body.clone(), headers: r.headers.clone() });
                let auth = r.header("authorization").unwrap_or_default().to_string();
                let reply = match service.as_str() {
                    "gitlab" | "jira" => {
                        let expected = if service == "gitlab" { GITLAB_TOKEN } else { JIRA_TOKEN };
                        if auth != format!("Bearer {expected}") {
                            Some(Resp::Json(401, json!({ "message": "401 Unauthorized" })))
                        } else if service == "gitlab" {
                            gitlab(&method, &path, &body, &base)
                        } else {
                            jira(&method, &path, &body)
                        }
                    }
                    "confluence" => confluence(&method, &path, &body, auth == format!("Bearer {CONFLUENCE_TOKEN}"), &version),
                    "chat" => {
                        if r.header("x-auth-token") != Some(ROCKETCHAT_TOKEN) || r.header("x-user-id") != Some(ROCKETCHAT_USER) {
                            Some(Resp::Json(401, json!({ "status": "error", "message": "You must be logged in to do this." })))
                        } else {
                            rocketchat(&method, &path, &body)
                        }
                    }
                    _ => None,
                };
                reply.unwrap_or_else(|| Resp::Json(404, json!({ "message": format!("Pas de route : {method} {path}") })))
            }
        })
    };
    let server = start(0, None, h).await;
    *base.lock() = server.url.clone();
    FakeServices { url: server.url.clone(), requests, _server: server }
}
