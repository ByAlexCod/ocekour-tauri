//! Outils web : lecture simple et pilotage d'un vrai navigateur (Chromium du poste, sans
//! interface). Ignoré si aucun Chromium n'est installé.

use super::support::{call, names, test_app};
use crate::testing::http::{Resp, handler, start};
use serde_json::json;

const PAGE: &str = r#"<!doctype html><html><head><title>Accueil</title></head><body>
<h1>Bienvenue</h1>
<input id="q" placeholder="Recherche" value="ancien">
<button onclick="document.getElementById('r').textContent = 'Résultat : ' + document.getElementById('q').value">Chercher</button>
<form onsubmit="event.preventDefault(); document.getElementById('r').textContent = 'Envoyé : ' + document.getElementById('f').value">
  <input id="f" aria-label="Formulaire">
</form>
<select id="s" aria-label="Couleur" onchange="document.getElementById('r').textContent = 'Couleur : ' + this.value">
  <option value="r">Rouge</option><option value="v">Vert</option>
</select>
<button onclick="alert('bloquant ?'); document.getElementById('r').textContent = 'Après alerte'">Alerte</button>
<button onclick="setTimeout(() => document.getElementById('r').textContent = 'Plus tard', 300)">Différé</button>
<a href="/suite">Page suivante</a>
<p id="r"></p>
<div style="display:none"><button>Caché</button></div>
</body></html>"#;

fn chromium_available() -> bool { crate::web::browser::candidates("chromium").iter().any(|p| p.is_file()) }

/// Ref d'un élément dans un snapshot (« - [e3] button « Chercher » »).
fn ref_of(snapshot: &str, label: &str) -> String {
    let line = snapshot.lines().find(|l| l.contains(label)).unwrap_or_else(|| panic!("{label} absent :\n{snapshot}"));
    line.split('[').nth(1).unwrap().split(']').next().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn navigateur() {
    if !chromium_available() {
        eprintln!("Chromium introuvable : test ignoré");
        return;
    }
    let server = start(0, None, handler(|r| async move {
        let html = |s: &str| Resp::Bytes(200, s.as_bytes().to_vec(), vec![("content-type".into(), "text/html; charset=utf-8".into())]);
        match r.path.as_str() {
            "/suite" => html("<title>Suite</title><h2>Deuxième page</h2>"),
            _ => html(PAGE),
        }
    })).await;
    let t = test_app("web:\n  enabled: false\n  browser: chromium\n  headless: true\n  timeoutMs: 5000\n", true).await;
    let app = &t.app;
    assert!(!names(app).await.iter().any(|n| n.starts_with("web_")));
    app.config.edit(|doc, _| { doc.set(&crate::ypath!["web", "enabled"], &json!(true))?; Ok(()) }).unwrap();
    assert!(names(app).await.contains(&"web_naviguer".to_string()));

    // Lecture simple.
    let fetched = call(app, "web_recuperer", json!({ "url": format!("{}/", server.url) })).await;
    assert!(fetched.value["contenu"].as_str().unwrap().starts_with("# Accueil"), "{}", fetched.text);
    assert!(call(app, "web_recuperer", json!({ "url": "ftp://x/y" })).await.error);

    // Snapshot.
    let snap = call(app, "web_naviguer", json!({ "url": format!("{}/", server.url) })).await;
    assert!(!snap.error, "{}", snap.text);
    let s = snap.text;
    assert!(s.starts_with("Page : Accueil\nURL : "), "{s}");
    assert!(s.contains("heading:1 « Bienvenue »"), "{s}");
    assert!(s.contains("input:text « Recherche » = « ancien »"), "{s}");
    assert!(!s.contains("Caché"), "{s}");

    // Saisie (remplace), clic.
    let field = ref_of(&s, "« Recherche »");
    let s = call(app, "web_saisir", json!({ "ref": field, "texte": "paiement" })).await.text;
    assert!(s.contains("= « paiement »"), "{s}");
    let s = call(app, "web_cliquer", json!({ "ref": ref_of(&s, "« Chercher »") })).await.text;
    let text = call(app, "web_texte", json!({})).await.text;
    assert!(text.contains("Résultat : paiement"), "{text}");

    // Saisie validée par Entrée.
    call(app, "web_saisir", json!({ "ref": ref_of(&s, "« Formulaire »"), "texte": "x", "valider": true })).await;
    assert!(call(app, "web_texte", json!({})).await.text.contains("Envoyé : x"));

    // Liste déroulante (libellé ou valeur).
    let s = call(app, "web_apercu", json!({})).await.text;
    let chosen = call(app, "web_choisir", json!({ "ref": ref_of(&s, "« Couleur »"), "valeurs": ["Vert"] })).await;
    assert!(!chosen.error, "{}", chosen.text);
    let t2 = call(app, "web_texte", json!({})).await.text;
    assert!(t2.contains("Couleur : v"), "{t2}\n{s}\n{}", chosen.text);
    assert!(call(app, "web_choisir", json!({ "ref": ref_of(&s, "« Couleur »"), "valeurs": ["Bleu"] })).await.error);

    // Alerte refusée automatiquement (la page n'est pas bloquée).
    call(app, "web_cliquer", json!({ "ref": ref_of(&s, "« Alerte »") })).await;
    assert!(call(app, "web_texte", json!({})).await.text.contains("Après alerte"));

    // Attente d'un texte.
    call(app, "web_cliquer", json!({ "ref": ref_of(&s, "« Différé »") })).await;
    let waited = call(app, "web_patienter", json!({ "texte": "plus  TARD" })).await;
    assert!(!waited.error, "{}", waited.text);
    assert!(call(app, "web_patienter", json!({})).await.error);
    let missing = call(app, "web_patienter", json!({ "texte": "jamais affiché" })).await;
    assert!(missing.error && missing.text.contains("non visible"), "{}", missing.text);

    // Ref inconnue : erreur explicite.
    let unknown = call(app, "web_cliquer", json!({ "ref": "e999" })).await;
    assert!(unknown.error && unknown.text.contains("introuvable"), "{}", unknown.text);

    // Touche clavier, lien, capture.
    assert!(!call(app, "web_touche", json!({ "touche": "Control+A" })).await.error);
    assert!(call(app, "web_touche", json!({ "touche": "Hyper+Z" })).await.error);
    let s = call(app, "web_apercu", json!({})).await.text;
    let s = call(app, "web_cliquer", json!({ "ref": ref_of(&s, "« Page suivante »") })).await.text;
    assert!(s.starts_with("Page : Suite"), "{s}");
    let session = super::support::session(app).await;
    let mut session = session;
    let shot = session.handle(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "web_capture", "arguments": { "pleine_page": true } } })).await.unwrap();
    let data = shot["result"]["content"][1]["data"].as_str().unwrap();
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
    assert_eq!(&png[1..4], b"PNG");

    // Onglets.
    call(app, "web_naviguer", json!({ "url": format!("{}/", server.url), "nouvel_onglet": true })).await;
    let tabs = call(app, "web_onglets", json!({})).await.value;
    let tabs = tabs["onglets"].as_array().unwrap();
    assert_eq!(tabs.len(), 2, "{tabs:?}");
    assert_eq!((tabs[0]["titre"].as_str(), tabs[1]["actif"].as_bool()), (Some("Suite"), Some(true)));
    assert_eq!(call(app, "web_fermer", json!({ "onglet": 1 })).await.text, "Onglet 1 fermé (1 restant).");
    assert!(call(app, "web_fermer", json!({ "onglet": 5 })).await.error);
    assert_eq!(call(app, "web_fermer", json!({ "onglet": 0 })).await.text, "Dernier onglet fermé : navigateur fermé.");
    assert!(!app.web.is_running().await);
    assert_eq!(call(app, "web_onglets", json!({})).await.value, json!({ "onglets": [] }));
    // Relancé au besoin.
    assert!(!call(app, "web_naviguer", json!({ "url": format!("{}/suite", server.url) })).await.error);
    assert_eq!(call(app, "web_fermer", json!({})).await.text, "Navigateur fermé.");
}
