//! Comptes (GitLab, Jira, Confluence, Rocket.Chat) et outils MCP des services, contre de
//! faux serveurs.

use super::support::{assert_match, call, names, test_app};
use crate::config::Tls;
use crate::services::journal::Query;
use crate::testing::fake_services::*;
use serde_json::json;

fn tls() -> Tls { Tls::default() }

#[tokio::test(flavor = "multi_thread")]
async fn comptes_et_outils_des_services() {
    let fake = start_fake_services().await;
    let t = test_app("# ma configuration\n", true).await;
    let app = &t.app;
    let accounts = &app.accounts;
    let has = |names: &[String], prefix: &str| names.iter().any(|n| n.starts_with(prefix));

    // Sans compte : aucun outil de service.
    let tools = names(app).await;
    assert!(tools.contains(&"logs_lire".to_string()));
    assert!(!tools.iter().any(|t| ["gitlab_", "jira_", "confluence_", "rocketchat_"].iter().any(|p| t.starts_with(p))));

    // URL dans config.yaml (commentaires gardés), jeton chiffré à part, test.
    accounts.save("gitlab", &format!("{}/gitlab/", fake.url), &tls()).unwrap();
    let text = t.read("config.yaml");
    assert!(text.contains("# ma configuration"), "{text}");
    assert!(text.contains(&format!("url: {}/gitlab\n", fake.url)), "{text}");
    assert_eq!(accounts.token_page("gitlab", false).unwrap(), format!("{}/gitlab/-/user_settings/personal_access_tokens?name=Ocekour&scopes=api", fake.url));
    let refused = accounts.set_token("gitlab", "mauvais", None).await.unwrap();
    assert!(!refused.ok);
    assert!(regex::Regex::new("Jeton GitLab refusé.*Comptes").unwrap().is_match(refused.error.as_deref().unwrap()), "{refused:?}");
    fake.clear();
    let result = accounts.set_token("gitlab", GITLAB_TOKEN, None).await.unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(result.identity.as_deref(), Some("Alex Martin (@amartin)"));
    assert!(result.warning.as_deref().unwrap().contains("« api » est recommandée"), "{result:?}");
    assert!(!t.read("config.yaml").contains(GITLAB_TOKEN));
    assert!(!t.read("secrets.json").contains(GITLAB_TOKEN));
    assert_eq!(fake.requests.lock()[0].headers.get("user-agent").map(String::as_str), Some("Ocekour/0.3.0 (agent MCP)"));
    let account = accounts.list().into_iter().find(|a| a.id == "gitlab").unwrap();
    assert_eq!((account.url.as_deref(), account.has_token, account.enabled, account.write), (Some(format!("{}/gitlab", fake.url).as_str()), true, true, false));

    // Lecture visible, écriture cachée tant qu'elle n'est pas autorisée.
    let tools = names(app).await;
    assert!(tools.contains(&"gitlab_mr_lire".into()) && !tools.contains(&"gitlab_mr_commenter".into()));
    accounts.set_flags("gitlab", None, Some(true)).unwrap();
    assert!(names(app).await.contains(&"gitlab_mr_commenter".into()));
    accounts.set_flags("gitlab", Some(false), None).unwrap();
    assert!(!has(&names(app).await, "gitlab_"));
    accounts.set_flags("gitlab", Some(true), None).unwrap();

    // GitLab : lecture de MR par URL, diff (ancienne API), log de job.
    let mr = call(app, "gitlab_mr_lire", json!({ "projet": format!("{}/gitlab/grp/app/-/merge_requests/12", fake.url) })).await;
    assert_match(&mr.value, &json!({ "iid": 12, "titre": "Paiement", "source": "feat" }));
    let diff = call(app, "gitlab_mr_diff", json!({ "projet": "grp/app", "mr": 12 })).await;
    assert_match(&diff.value[0], &json!({ "nouveau": "src/a.ts", "diff": FAKE_DIFF }));
    assert_eq!(diff.value.as_array().unwrap().len(), 1);
    let log = call(app, "gitlab_job_log", json!({ "projet": "grp/app", "job": 5, "fin": 2 })).await;
    assert_eq!(log.text, "ERREUR finale\n");

    // GitLab : commentaire positionné, signé, journalisé.
    let c = call(app, "gitlab_mr_commenter", json!({
        "projet": "grp/app", "mr": 12, "texte": "Pourquoi **deux bis** ?", "position": { "fichier": "src/a.ts", "ligne": 2 },
    })).await;
    assert!(!c.error, "{}", c.text);
    assert_match(&c.value, &json!({ "fil": "d1", "positionne": true }));
    let post = fake.last_write().unwrap();
    assert_eq!(post.path, "/gitlab/api/v4/projects/grp%2Fapp/merge_requests/12/discussions");
    assert_eq!(post.body, json!({
        "body": "Pourquoi **deux bis** ?\n\n_— 🤖 via Ocekour_",
        "position": { "position_type": "text", "base_sha": "b", "start_sha": "s", "head_sha": "h", "old_path": "src/a.ts", "new_path": "src/a.ts", "new_line": 2 },
    }));
    let (entries, _) = app.journal.list(&Query::default());
    assert_match(&serde_json::to_value(&entries[0]).unwrap(), &json!({
        "service": "gitlab", "tool": "gitlab_mr_commenter", "target": "grp/app !12 src/a.ts:2", "status": "ok",
        "client": "test-agent 9.9", "url": format!("{}/gitlab/grp/app/-/merge_requests/12#note_101", fake.url),
    }));

    // GitLab : ligne hors diff → erreur, aussi journalisée.
    let c = call(app, "gitlab_mr_commenter", json!({ "projet": "grp/app", "mr": 12, "texte": "x", "position": { "fichier": "src/a.ts", "ligne": 99 } })).await;
    assert!(c.error && c.text.contains("hors des modifications"), "{}", c.text);
    let (errors, _) = app.journal.list(&Query { status: Some("error".into()), ..Default::default() });
    assert_eq!((errors[0].tool.as_str(), errors[0].status.as_str()), ("gitlab_mr_commenter", "error"));

    // Jira : ticket lisible, commentaire Markdown converti et signé, transition par nom.
    accounts.save("jira", &format!("{}/jira", fake.url), &tls()).unwrap();
    accounts.set_token("jira", JIRA_TOKEN, None).await.unwrap();
    accounts.set_flags("jira", None, Some(true)).unwrap();
    let ticket = call(app, "jira_ticket_lire", json!({ "ticket": format!("{}/jira/browse/PAY-1", fake.url) })).await;
    assert_match(&ticket.value, &json!({
        "cle": "PAY-1", "statut": "Ouvert", "champs": { "Criticité (customfield_10010)": "Haute" },
        "commentaires": [{ "auteur": "Bob (bob)", "texte": "Vu" }], "url": format!("{}/jira/browse/PAY-1", fake.url),
    }));
    call(app, "jira_commenter", json!({ "ticket": "PAY-1", "texte": "Corrigé par **!12**" })).await;
    assert_eq!(fake.last_write().unwrap().body, json!({ "body": "Corrigé par *!12*\n\n_— via Ocekour (agent IA)_" }));
    let tr = call(app, "jira_transition", json!({ "ticket": "PAY-1", "transition": "en cours" })).await;
    assert_match(&tr.value, &json!({ "statut": "En cours" }));
    assert_eq!(fake.last_write().unwrap().body, json!({ "transition": { "id": "31" } }));

    // Confluence : jeton ignoré détecté ; page en Markdown ; modification avec contrôle de version.
    accounts.save("confluence", &format!("{}/confluence", fake.url), &tls()).unwrap();
    let anonymous = accounts.set_token("confluence", "faux", None).await.unwrap();
    assert!(!anonymous.ok && anonymous.error.as_deref().unwrap().contains("anonyme"), "{anonymous:?}");
    accounts.set_token("confluence", CONFLUENCE_TOKEN, None).await.unwrap();
    accounts.set_flags("confluence", None, Some(true)).unwrap();
    let page = call(app, "confluence_page_lire", json!({ "page": "123" })).await;
    assert_match(&page.value, &json!({
        "titre": "Déploiement", "version": 4, "url": format!("{}/confluence/display/DEV/D%C3%A9ploiement", fake.url),
        "contenu": "# Déployer\n\nLancer :\n\n```bash\nmake deploy ENV=prod\n```",
    }));
    let stale = call(app, "confluence_page_modifier", json!({ "page": "123", "version": 3, "contenu": "x" })).await;
    assert!(stale.error && stale.text.contains("a changé depuis sa lecture"), "{}", stale.text);
    let done = call(app, "confluence_page_modifier", json!({ "page": "123", "version": 4, "contenu": "# Nouveau" })).await;
    assert!(!done.error, "{}", done.text);
    let put = fake.requests.lock().iter().find(|r| r.method == "PUT").cloned().unwrap();
    assert_match(&put.body, &json!({
        "version": { "number": 5, "message": "via Ocekour (agent IA)" },
        "body": { "storage": { "value": "<h1>Nouveau</h1>", "representation": "storage" } },
    }));
    let last = fake.last_write().unwrap();
    assert_eq!((last.path.as_str(), &last.body), ("/confluence/rest/api/content/123/label", &json!([{ "prefix": "global", "name": "ocekour" }])));

    // Rocket.Chat : identifiant + jeton, historique, message signé.
    accounts.save("rocketchat", &format!("{}/chat", fake.url), &tls()).unwrap();
    let e = accounts.set_token("rocketchat", "x", None).await.unwrap_err();
    assert!(e.to_string().contains("identifiant"));
    assert!(accounts.set_token("rocketchat", ROCKETCHAT_TOKEN, Some(ROCKETCHAT_USER)).await.unwrap().ok);
    accounts.set_flags("rocketchat", None, Some(true)).unwrap();
    let h = call(app, "rocketchat_historique", json!({ "salon": "#general" })).await;
    assert_match(&h.value, &json!({ "salon": format!("{}/chat/channel/general", fake.url), "messages": [{ "id": "M1", "auteur": "bob", "texte": "Bonjour" }] }));
    let sent = call(app, "rocketchat_envoyer", json!({ "salon": "@bob", "texte": "Build vert ✅" })).await;
    assert_match(&sent.value, &json!({ "message": "M9", "url": format!("{}/chat/direct/D1?msg=M9", fake.url) }));
    assert_eq!(fake.last_write().unwrap().body, json!({ "roomId": "D1", "text": "Build vert ✅\n\n_— 🤖 via Ocekour_" }));

    // Jeton oublié : outils retirés ; secrets jamais dans les réponses.
    let all = serde_json::to_string(&app.journal.list(&Query::default()).0).unwrap();
    for secret in [GITLAB_TOKEN, JIRA_TOKEN, ROCKETCHAT_TOKEN] { assert!(!all.contains(secret)); }
    accounts.forget_token("rocketchat").unwrap();
    assert!(!has(&names(app).await, "rocketchat_"));
    assert!(!call(app, "configuration_lire", json!({})).await.text.contains(JIRA_TOKEN));

    // Préconfiguration de clé : services absents ajoutés, existants gardés.
    use crate::licence::preset::validate;
    let added = app.connections.add_preset(&validate(Some(&json!({ "services": {
        "jira": { "url": "https://autre-jira.intra" }, "rocketchat": { "url": "https://chat.intra", "tls": { "insecure": true } },
    }}))).unwrap()).unwrap();
    assert!(added.services.is_empty());
    accounts.save("rocketchat", "", &tls()).unwrap();
    let again = app.connections.add_preset(&validate(Some(&json!({ "services": { "rocketchat": { "url": "https://chat.intra", "tls": { "insecure": true } } } }))).unwrap()).unwrap();
    assert_eq!(again.services, vec!["rocketchat".to_string()]);
    let config = app.config.config();
    let rc = config.services.get("rocketchat").unwrap();
    assert_eq!((rc.url.as_str(), rc.tls.insecure, rc.write), ("https://chat.intra", true, false));
    assert_eq!(config.services.get("jira").unwrap().url, format!("{}/jira", fake.url));
}
