//! Outils MCP « Web / navigateur ». Visibles seulement quand web.enabled. Modèle « snapshot
//! d'accessibilité » : les actions ciblent une ref (eN) renvoyée par le snapshot.

use super::fetch::fetch_web;
use crate::mcp::{Args, Tool, Visibility, fail, image, ok, tool};
use crate::obj;
use crate::schema::{self as z};
use serde_json::json;

fn url_s(d: &str) -> crate::schema::S {
    z::string().url().refine(|v| {
        let s = v.as_str().unwrap_or_default();
        (!(s.starts_with("http://") || s.starts_with("https://"))).then(|| "URL http(s) attendue".to_string())
    }).describe(d)
}

pub fn tools() -> Vec<Tool> {
    let w = |t: Tool| t.visible(Visibility::Web);
    vec![
        w(tool("web_recuperer", "Récupérer une page web (sans navigateur)",
            "GET HTTP(S) d’une URL et renvoie le contenu en Markdown (HTML) ou en texte. Rapide, sans navigateur : idéal pour lire une doc ou une page statique. Pour une page qui exige JavaScript ou une interaction, utilisez web_naviguer.")
            .input(vec![("url", url_s("URL http(s) à récupérer"))])
            .read_only()
            .run(|app, a, _| async move {
                let res = fetch_web(&a.string("url"), &app.config.config().web.tls, &app.web.user_agent).await?;
                Ok(ok(obj! { "url" => res.url, "statut" => res.status, "type" => res.content_type, "tronque" => res.truncated.then_some(true), "contenu" => res.body }))
            })),
        w(tool("web_naviguer", "Ouvrir / charger une page",
            "Ouvre une URL dans le navigateur (lancé au besoin) et renvoie un snapshot d’accessibilité : chaque élément a une ref [eN] à cibler avec web_cliquer / web_saisir.")
            .input(vec![("url", url_s("URL http(s) à ouvrir")), ("nouvel_onglet", z::boolean().optional().describe("Ouvrir dans un nouvel onglet"))])
            .run(|app, a, _| async move { Ok(ok(app.web.goto(&a.string("url"), a.bool("nouvel_onglet").unwrap_or(false)).await?)) })),
        w(tool("web_apercu", "Snapshot de la page", "Snapshot d’accessibilité de la page active (éléments et refs à jour).")
            .read_only()
            .run(|app, _, _| async move { Ok(ok(app.web.snapshot().await?)) })),
        w(tool("web_texte", "Texte de la page", "Contenu visible de la page active, en texte brut.")
            .read_only()
            .run(|app, _, _| async move { Ok(ok(app.web.text().await?)) })),
        w(tool("web_cliquer", "Cliquer un élément", "Clique l’élément désigné par sa ref (ex. e12) ; renvoie le nouveau snapshot.")
            .input(vec![
                ("ref", z::string().min(1).describe("Ref de l’élément (eN, voir le snapshot)")),
                ("description", z::string().optional().describe("Libellé lisible (traçabilité, facultatif)")),
            ])
            .run(|app, a, _| async move { Ok(ok(app.web.click(&a.string("ref")).await?)) })),
        w(tool("web_saisir", "Saisir dans un champ", "Remplit un champ (ref) avec un texte ; `valider` appuie sur Entrée ensuite. Renvoie le nouveau snapshot.")
            .input(vec![
                ("ref", z::string().min(1).describe("Ref du champ (eN)")),
                ("texte", z::string().describe("Texte à saisir (remplace le contenu)")),
                ("valider", z::boolean().optional().describe("Appuyer sur Entrée après la saisie")),
            ])
            .run(|app, a, _| async move { Ok(ok(app.web.type_text(&a.string("ref"), &a.string("texte"), a.bool("valider").unwrap_or(false)).await?)) })),
        w(tool("web_choisir", "Choisir dans une liste", "Sélectionne une ou plusieurs options d’un <select> (ref).")
            .input(vec![
                ("ref", z::string().min(1).describe("Ref du <select> (eN)")),
                ("valeurs", z::array(z::string()).min(1).describe("Valeurs ou libellés des options")),
            ])
            .run(|app, a, _| async move {
                let values: Vec<String> = a["valeurs"].as_array().unwrap().iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
                Ok(ok(app.web.select(&a.string("ref"), &values).await?))
            })),
        w(tool("web_touche", "Touche clavier", "Appuie une touche (Enter, Escape, Tab, PageDown, ArrowDown, Control+A…).")
            .input(vec![("touche", z::string().min(1).describe("Nom de touche Playwright"))])
            .run(|app, a, _| async move { Ok(ok(app.web.press(&a.string("touche")).await?)) })),
        w(tool("web_patienter", "Attendre", "Attend qu’un texte soit visible à l’écran, et/ou un délai (ms, 60 s au plus).")
            .input(vec![
                ("texte", z::string().optional().describe("Texte à attendre à l’écran")),
                ("ms", z::int().min(0).max(60_000).optional().describe("Délai en millisecondes")),
            ])
            .read_only()
            .run(|app, a, _| async move {
                let (text, ms) = (a.str("texte").filter(|t| !t.is_empty()), a.int("ms").filter(|m| *m > 0));
                if text.is_none() && ms.is_none() { return Ok(fail("Indiquez un texte à attendre et/ou un délai (ms).")); }
                Ok(ok(app.web.wait_for(text.as_deref(), ms.map(|m| m as u64)).await?))
            })),
        w(tool("web_capture", "Capture d’écran", "Capture la page active (PNG). `pleine_page` capture toute la hauteur.")
            .input(vec![("pleine_page", z::boolean().optional().describe("Capturer toute la page"))])
            .read_only()
            .run(|app, a, _| async move { Ok(image(app.web.screenshot(a.bool("pleine_page").unwrap_or(false)).await?, "image/png", Some("Capture de la page active."))) })),
        w(tool("web_onglets", "Lister les onglets", "Liste les onglets ouverts (index, titre, URL, onglet actif).")
            .read_only()
            .run(|app, _, _| async move { Ok(ok(json!({ "onglets": app.web.tabs().await? }))) })),
        w(tool("web_fermer", "Fermer un onglet / le navigateur", "Ferme l’onglet d’index donné, ou tout le navigateur si aucun index.")
            .input(vec![("onglet", z::int().min(0).optional().describe("Index de l’onglet à fermer"))])
            .destructive()
            .run(|app, a, _| async move { Ok(ok(app.web.close(a.int("onglet").map(|i| i as usize)).await?)) })),
    ]
}
