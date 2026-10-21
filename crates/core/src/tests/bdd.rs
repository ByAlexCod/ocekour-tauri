//! Bases de données : PostgreSQL réel (BDD_TEST_PG=hôte:port, utilisateur lecteur/lecteur,
//! table `clients`, vue `v_clients`), garde lecture seule, outils MCP.

use super::support::{assert_match, call, names, test_app};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn postgres_en_lecture_seule() {
    let Ok(address) = std::env::var("BDD_TEST_PG") else {
        eprintln!("BDD_TEST_PG absent : test ignoré");
        return;
    };
    let (host, port) = address.split_once(':').unwrap();
    let t = test_app("# bases\n", true).await;
    let app = &t.app;
    assert!(!names(app).await.iter().any(|n| n.starts_with("bdd_")));

    let draft = json!({ "engine": "postgres", "name": "Recette", "host": host, "port": port.parse::<u16>().unwrap(), "database": "postgres", "user": "lecteur" });
    // Test sans mot de passe enregistré, mauvais mot de passe, bon mot de passe.
    assert!(!app.databases.test(&draft).await.ok);
    let mut bad = draft.clone();
    bad["password"] = json!("faux");
    let refused = app.databases.test(&bad).await;
    assert!(!refused.ok && refused.error.is_some(), "{refused:?}");
    let mut good = draft.clone();
    good["password"] = json!("lecteur");
    let tested = app.databases.test(&good).await;
    assert!(tested.ok && tested.summary.as_deref().unwrap().starts_with("PostgreSQL"), "{tested:?}");
    assert!(app.databases.save(&draft).is_err(), "mot de passe obligatoire");
    let id = app.databases.save(&good).unwrap();
    assert!(id.starts_with("db-"));
    let yaml = t.read("config.yaml");
    assert!(yaml.contains("# bases") && yaml.contains("engine: postgres") && !yaml.contains("lecteur\n  password"), "{yaml}");
    assert!(!yaml.contains("password"), "{yaml}");
    assert_match(&serde_json::to_value(app.databases.list()).unwrap(), &json!([{ "id": id, "port": port.parse::<u16>().unwrap(), "hasSecret": true }]));

    // Outils visibles.
    assert!(names(app).await.contains(&"bdd_requete".to_string()));
    let tables = call(app, "bdd_tables", json!({ "connexion": id })).await;
    assert_match(&tables.value, &json!({ "tables": [{ "schema": "public", "name": "clients", "kind": "table" }, { "schema": "public", "name": "v_clients", "kind": "view" }] }));
    let cols = call(app, "bdd_colonnes", json!({ "connexion": id, "table": "clients" })).await;
    assert_match(&cols.value, &json!({ "colonnes": [{ "name": "id", "type": "integer", "nullable": false }, { "name": "nom", "type": "text", "nullable": true }, { "name": "cree", "nullable": true }] }));
    let preview = call(app, "bdd_apercu", json!({ "connexion": id, "table": "clients", "schema": "public", "limite": 1 })).await;
    assert_match(&preview.value, &json!({ "columns": ["id", "nom", "cree"], "rowCount": 1, "truncated": false }));
    assert_eq!((&preview.value["rows"][0][0], &preview.value["rows"][0][1]), (&json!(1), &json!("Alice")));
    assert!(preview.value["rows"][0][2].as_str().unwrap().ends_with('Z'));
    let q = call(app, "bdd_requete", json!({ "connexion": id, "sql": "SELECT id, nom, id > 1 AS grand, NULL AS rien FROM v_clients ORDER BY id;", "limite": 1 })).await;
    assert_match(&q.value, &json!({ "columns": ["id", "nom", "grand", "rien"], "rows": [[1, "Alice", false, null]], "rowCount": 2, "truncated": true }));

    // Écritures refusées : par la garde, puis par la transaction en lecture seule du moteur.
    let w = call(app, "bdd_requete", json!({ "connexion": id, "sql": "DELETE FROM clients" })).await;
    assert!(w.error && w.text.contains("Seules les lectures"), "{}", w.text);
    let sneaky = call(app, "bdd_requete", json!({ "connexion": id, "sql": "WITH x AS (SELECT 1) SELECT pg_catalog.nextval('seq_inexistante')" })).await;
    assert!(sneaky.error, "{}", sneaky.text);
    let func = call(app, "bdd_requete", json!({ "connexion": id, "sql": "SELECT pg_catalog.set_config('application_name', 'x', false)" })).await;
    assert!(!func.error || func.text.contains("read-only"), "{}", func.text);

    // Plan d'exécution.
    let plan = call(app, "bdd_expliquer", json!({ "connexion": id, "sql": "SELECT * FROM clients" })).await;
    assert!(plan.value["columns"][0] == "QUERY PLAN", "{}", plan.text);

    // Erreur SQL lisible.
    let e = call(app, "bdd_requete", json!({ "connexion": id, "sql": "SELECT * FROM inconnue" })).await;
    assert!(e.error && e.text.contains("inconnue"), "{}", e.text);

    // Modification (sans nouveau mot de passe), puis suppression.
    let mut edit = draft.clone();
    edit["id"] = json!(id);
    edit["name"] = json!("Recette 2");
    assert_eq!(app.databases.save(&edit).unwrap(), id);
    assert_eq!(app.databases.list()[0].name, "Recette 2");
    app.databases.delete(&id).unwrap();
    assert!(app.databases.list().is_empty() && !app.databases.has_secret(&id));
    assert!(!names(app).await.iter().any(|n| n.starts_with("bdd_")));
}
