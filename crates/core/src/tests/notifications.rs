//! Notifications : lecteurs de source (GitLab, Jira, Confluence) et boucle d'envoi, contre un
//! faux serveur dont les données changent au fil du test.

use crate::config::{ConfigStore, Tls};
use crate::notifications::message::build_message;
use crate::notifications::state::{NotifItem, NotificationState};
use crate::notifications::{Clients, Notifier, confluence, gitlab, jira};
use crate::services::http::{ServiceError, ServiceHttp};
use crate::testing::http::{Req, Resp, TestServer, handler, start};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
struct Fake {
    todos: Vec<Value>,
    mrs_authored: Vec<Value>,
    mrs_reviewer: Vec<Value>,
    discussions: HashMap<String, Vec<Value>>,
    jira_issues: Vec<Value>,
    confluence_mentions: Vec<Value>,
    confluence_watched: Vec<Value>,
    sent: Vec<String>,
    fail: bool,
}

fn todo(id: i64, action: &str, kind: &str, iid: i64, title: &str) -> Value {
    json!({ "id": id, "action_name": action, "target_type": kind, "target": { "iid": iid, "title": title, "web_url": format!("http://x/{iid}") },
        "project": { "path_with_namespace": "grp/app" }, "author": { "username": "bob" } })
}

fn future() -> String { crate::text::iso(chrono::Utc::now() + chrono::Duration::hours(1)) }
fn past() -> String { crate::text::iso(chrono::Utc::now() - chrono::Duration::hours(1)) }

async fn server(fake: Arc<Mutex<Fake>>) -> TestServer {
    start(0, None, handler(move |r: Req| {
        let fake = fake.clone();
        async move {
            let mut f = fake.lock();
            let mut parts = r.path.splitn(3, '/').skip(1);
            let service = parts.next().unwrap_or_default().to_string();
            let rest = format!("/{}", parts.next().unwrap_or_default());
            let ok = |v: Value| Resp::Json(200, v);
            match (service.as_str(), rest.as_str()) {
                ("gitlab", "/api/v4/todos") => ok(json!(f.todos)),
                ("gitlab", "/api/v4/user") => ok(json!({ "username": "amartin", "id": 1 })),
                ("gitlab", "/api/v4/merge_requests") => ok(json!(if r.q("reviewer_username").is_some() { &f.mrs_reviewer } else { &f.mrs_authored })),
                ("gitlab", p) if p.ends_with("/discussions") => {
                    let seg: Vec<&str> = p.split('/').collect();
                    ok(json!(f.discussions.get(&format!("{}!{}", seg[4], seg[6])).cloned().unwrap_or_default()))
                }
                ("jira", "/rest/api/2/myself") => ok(json!({ "name": "amartin" })),
                ("jira", "/rest/api/2/search") => ok(json!({ "issues": f.jira_issues })),
                ("confluence", "/rest/api/user/current") => ok(json!({ "type": "known", "username": "amartin" })),
                ("confluence", "/rest/api/content/search") => {
                    ok(json!({ "results": if r.q("cql").unwrap_or_default().contains("mention") { &f.confluence_mentions } else { &f.confluence_watched } }))
                }
                ("rocketchat", "/api/v1/me") => ok(json!({ "username": "amartin" })),
                ("rocketchat", "/api/v1/chat.postMessage") => {
                    if f.fail { return Resp::Json(500, json!({ "success": false, "error": "indisponible" })); }
                    f.sent.push(r.json()["text"].as_str().unwrap_or_default().to_string());
                    ok(json!({ "message": { "_id": "M1", "rid": "D1" } }))
                }
                _ => Resp::Json(404, json!({})),
            }
        }
    }))
    .await
}

fn http_for(base: &str, service: &str) -> ServiceHttp {
    ServiceHttp { label: service.into(), url: format!("{base}/{service}"), tls: Tls::default(), auth: vec![], user_agent: "test".into() }
}

struct AllAvailable(String);
impl Clients for AllAvailable {
    fn available(&self, _: &str) -> bool { true }
    fn client(&self, id: &str) -> Result<ServiceHttp, ServiceError> { Ok(http_for(&self.0, id)) }
}

fn ev(list: &[&str]) -> Vec<String> { list.iter().map(|s| s.to_string()).collect() }

#[test]
fn aides() {
    let now = crate::text::now_ms();
    assert_eq!(jira::relative_minutes(None, now), 2);
    assert_eq!(jira::relative_minutes(Some(&crate::text::iso(chrono::Utc::now() - chrono::Duration::minutes(5))), now), 7);
    assert_eq!(jira::relative_minutes(Some(&crate::text::iso(chrono::Utc::now() - chrono::Duration::days(30))), now), 7 * 24 * 60);
    assert!(jira::mentions("Merci [~amartin] de regarder", "amartin"));
    assert!(jira::mentions("cc [~accountid:amartin]", "amartin"));
    assert!(!jira::mentions("rien", "amartin"));
    assert_eq!(build_message(&[]), None);
    let items = vec![
        NotifItem { source: "gitlab".into(), kind: "review_requested".into(), title: "grp/app !42".into(), url: Some("http://x/42".into()), actor: Some("bob".into()), dedup_key: "g1".into(), ..Default::default() },
        NotifItem { source: "jira".into(), kind: "comment".into(), title: "PAY-1 « Bug »".into(), url: Some("http://x/PAY-1".into()), actor: Some("claire".into()), dedup_key: "j1".into(), ..Default::default() },
    ];
    let msg = build_message(&items).unwrap();
    assert!(msg.contains("2 nouveautés") && msg.contains("**GitLab**") && msg.contains("**Jira**"), "{msg}");
    assert!(msg.contains("Revue demandée — [grp/app !42](http://x/42) · @bob"), "{msg}");
}

#[tokio::test]
async fn lecteurs_de_source() {
    let fake = Arc::new(Mutex::new(Fake { todos: vec![todo(10, "review_requested", "MergeRequest", 42, "Paiement")], ..Default::default() }));
    let s = server(fake.clone()).await;
    let gl = http_for(&s.url, "gitlab");

    // GitLab : to-do plus récents, filtrés par type.
    let (items, last) = gitlab::fetch_todos(&gl, 5, &ev(&["review_requested"])).await.unwrap();
    assert_eq!(last, 10);
    assert_eq!((items.len(), items[0].title.as_str(), items[0].kind.as_str()), (1, "grp/app !42 « Paiement »", "review_requested"));
    assert_eq!(items[0].entity.as_ref().unwrap().key, "grp/app!42");
    assert!(gitlab::fetch_todos(&gl, 10, &ev(&["review_requested"])).await.unwrap().0.is_empty());

    // GitLab : commentaire sur ma MR et réponse à mon commentaire ; ignore les miens et les mentions.
    {
        let mut f = fake.lock();
        f.mrs_authored = vec![json!({ "project_id": 9, "iid": 42, "title": "Paiement", "web_url": "http://x/mr/42", "references": { "full": "grp/app!42" }, "author": { "username": "amartin" } })];
        f.mrs_reviewer = f.mrs_authored.clone();
        f.discussions.insert("9!42".into(), vec![
            json!({ "notes": [{ "id": 100, "author": { "username": "bob" }, "body": "Beau boulot", "created_at": future() }] }),
            json!({ "notes": [{ "id": 50, "author": { "username": "amartin" }, "body": "une question", "created_at": past() }, { "id": 101, "author": { "username": "claire" }, "body": "voici", "created_at": future() }] }),
            json!({ "notes": [{ "id": 102, "author": { "username": "amartin" }, "body": "moi", "created_at": future() }] }),
            json!({ "notes": [{ "id": 103, "author": { "username": "dan" }, "body": "cc @amartin", "created_at": future() }, { "id": 104, "system": true, "author": { "username": "dan" }, "body": "a approuvé", "created_at": future() }] }),
        ]);
    }
    let (notes, _) = gitlab::fetch_notes(&gl, None, &ev(&["mr_comment", "reply"]), &crate::text::now_iso()).await.unwrap();
    let got: Vec<(&str, &str, &str)> = notes.iter().map(|n| (n.kind.as_str(), n.actor.as_deref().unwrap(), n.dedup_key.as_str())).collect();
    assert_eq!(got, vec![("mr_comment", "bob", "gitlab:note:100"), ("reply", "claire", "gitlab:note:101")]);
    assert_eq!((notes[0].title.as_str(), notes[0].url.as_deref()), ("grp/app!42 « Paiement »", Some("http://x/mr/42#note_100")));

    // Jira : commentaire et mention d'un autre, ignore mes actions.
    fake.lock().jira_issues = vec![json!({ "key": "PAY-1", "fields": { "summary": "Bug", "comment": { "comments": [
        { "id": "1", "author": { "name": "bob" }, "body": "Regarde [~amartin]", "created": future() },
        { "id": "2", "author": { "name": "amartin" }, "body": "ok", "created": future() },
    ] } }, "changelog": { "histories": [
        { "id": "7", "author": { "name": "claire" }, "created": future(), "items": [{ "field": "status", "fromString": "Ouvert", "toString": "En cours" }, { "field": "assignee", "to": "amartin" }] },
    ] } })];
    let (items, _) = jira::fetch(&http_for(&s.url, "jira"), None, &ev(&["comment", "mention"]), "1970-01-01T00:00:00.000Z").await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].kind.as_str(), items[0].title.as_str(), items[0].actor.as_deref()), ("mention", "PAY-1 « Bug »", Some("bob")));
    let (all, _) = jira::fetch(&http_for(&s.url, "jira"), None, &ev(&["comment", "mention", "status", "assigned"]), "1970-01-01T00:00:00.000Z").await.unwrap();
    let kinds: Vec<&str> = all.iter().map(|i| i.kind.as_str()).collect();
    assert_eq!(kinds, vec!["status", "assigned", "mention"]);
    assert_eq!(all[0].body.as_deref(), Some("Ouvert → En cours"));

    // Confluence : la mention l'emporte, mes modifications ignorées.
    {
        let mut f = fake.lock();
        let page = json!({ "id": "100", "type": "page", "title": "Déploiement", "space": { "key": "DEV" }, "version": { "number": 4, "when": future(), "by": { "username": "claire" } }, "_links": { "webui": "/x" } });
        f.confluence_mentions = vec![page.clone()];
        f.confluence_watched = vec![page, json!({ "id": "200", "type": "page", "title": "Moi", "version": { "number": 1, "when": future(), "by": { "username": "amartin" } } })];
    }
    let (items, _) = confluence::fetch(&http_for(&s.url, "confluence"), None, &ev(&["mention", "update"]), "1970-01-01T00:00:00.000Z").await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].kind.as_str(), items[0].title.as_str(), items[0].actor.as_deref()), ("mention", "Déploiement (DEV)", Some("claire")));
}

struct Ctx {
    fake: Arc<Mutex<Fake>>,
    _server: TestServer,
    base: String,
    dir: tempfile::TempDir,
}

async fn ctx() -> Ctx {
    let fake = Arc::new(Mutex::new(Fake { todos: vec![todo(10, "review_requested", "MergeRequest", 42, "Paiement")], ..Default::default() }));
    let s = server(fake.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.yaml"), "notifications:\n  enabled: true\n  intervalSeconds: 60\n").unwrap();
    Ctx { fake, base: s.url.clone(), _server: s, dir }
}

impl Ctx {
    fn notifier(&self, desktop: Option<Arc<Mutex<Vec<(String, String)>>>>) -> (Arc<Notifier>, Arc<Mutex<Vec<(String, Option<f64>)>>>) {
        let config = ConfigStore::new(self.dir.path().join("config.yaml"));
        let state = NotificationState::new(self.dir.path().join("notifications.json"));
        let events: Arc<Mutex<Vec<(String, Option<f64>)>>> = Arc::default();
        let e2 = events.clone();
        let n = Notifier::new(
            Arc::new(AllAvailable(self.base.clone())),
            config,
            state,
            Arc::new(move |e| e2.lock().push((e.action.clone(), e.value))),
            Arc::new(|| true),
            desktop.map(|d| Arc::new(move |t: &str, b: &str| d.lock().push((t.into(), b.into()))) as crate::notifications::DesktopNotify),
        );
        (n, events)
    }
    fn sent(&self) -> Vec<String> { self.fake.lock().sent.clone() }
    fn push_todo(&self, t: Value) { self.fake.lock().todos.push(t); }
}

#[tokio::test]
async fn premier_passage_confirmation_sans_historique() {
    let c = ctx().await;
    let (n, _) = c.notifier(None);
    n.tick().await;
    assert_eq!(c.sent().len(), 1);
    assert!(c.sent()[0].contains("notifications activées"));
    n.tick().await;
    assert_eq!(c.sent().len(), 1, "le to-do existant est la base");
    assert!(c.dir.path().join("notifications.json").exists());
}

#[tokio::test]
async fn nouveaute_groupee_puis_anti_doublon() {
    let c = ctx().await;
    let (n, events) = c.notifier(None);
    n.tick().await;
    c.push_todo(todo(11, "assigned", "Issue", 7, "Fuite"));
    n.tick().await;
    assert_eq!(c.sent().len(), 2);
    assert!(c.sent()[1].contains("Assigné — [grp/app #7 « Fuite »](http://x/7) · @bob"), "{}", c.sent()[1]);
    assert!(events.lock().contains(&("gitlab".to_string(), Some(1.0))));
    n.tick().await;
    assert_eq!(c.sent().len(), 2);
    let status = n.status();
    assert_eq!((status.last_count, status.enabled), (Some(0), true));
    // État relu par un nouveau scrutateur : pas de nouvelle confirmation.
    let (again, _) = c.notifier(None);
    again.tick().await;
    assert_eq!(c.sent().len(), 2);
}

#[tokio::test]
async fn rocketchat_injoignable_regroupe_au_passage_suivant() {
    let c = ctx().await;
    let (n, _) = c.notifier(None);
    n.tick().await;
    c.push_todo(todo(12, "assigned", "Issue", 8, "X"));
    c.fake.lock().fail = true;
    n.tick().await;
    assert_eq!(c.sent().len(), 1);
    assert!(n.status().last_error.is_some());
    c.fake.lock().fail = false;
    n.tick().await;
    assert_eq!(c.sent().len(), 2);
    assert!(c.sent()[1].contains("#8"));
}

#[tokio::test]
async fn message_d_essai_et_notifications_systeme() {
    let c = ctx().await;
    let desktop: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let (n, _) = c.notifier(Some(desktop.clone()));
    assert_eq!(n.test().await, crate::notifications::TestOutcome { ok: true, error: None });
    assert!(c.sent()[0].contains("message d’essai"));
    assert!(desktop.lock()[0].0.contains("message d’essai"));

    n.tick().await;
    assert!(desktop.lock().last().unwrap().0.contains("notifications activées"));
    c.push_todo(todo(20, "assigned", "Issue", 9, "Fuite"));
    n.tick().await;
    let (title, body) = desktop.lock().last().unwrap().clone();
    assert_eq!(title, "Ocekour — 1 nouveauté");
    assert!(body.contains("GitLab — Assigné : grp/app #9 « Fuite » (@bob)"), "{body}");
}

#[tokio::test(start_paused = true)]
async fn boucle_programmee() {
    let c = ctx().await;
    let (n, _) = c.notifier(None);
    assert!(!n.status().running);
    n.apply();
    assert!(n.status().running);
    n.stop();
    assert!(!n.status().running);
}
