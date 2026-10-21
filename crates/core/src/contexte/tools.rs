//! Outils MCP de la mémoire de contexte (lecture seule), visibles quand le contexte IA est
//! activé. L'agent ne lit pas les fichiers : tout passe par ici.

use crate::mcp::{Args, Tool, Visibility, fail, ok, tool};
use crate::schema::{self as z};

pub fn tools() -> Vec<Tool> {
    let c = |t: Tool| t.visible(Visibility::Contexte).read_only();
    vec![
        c(tool("contexte_chercher", "Chercher dans le contexte",
            "Recherche plein-texte (mots-clés) dans la mémoire de contexte Ocekour (messages Rocket.Chat reçus, nouveautés GitLab / Jira / Confluence). Renvoie extraits et références, les plus récents d’abord.")
            .input(vec![
                ("motscles", z::string().min(1).describe("Mots-clés (tous requis)")),
                ("limite", z::int().min(1).max(50).optional().describe("Résultats au plus (défaut : 20)")),
            ])
            .run(|app, a, _| async move { Ok(ok(app.context_store.search(&a.string("motscles"), a.int("limite").unwrap_or(20) as usize))) })),
        c(tool("contexte_recents", "Contexte récent", "Les entités récemment mises à jour dans la mémoire de contexte (sommaire d’index.md).")
            .input(vec![("limite", z::int().min(1).max(100).optional().describe("Entités au plus (défaut : 30)"))])
            .run(|app, a, _| async move { Ok(ok(app.context_store.recents(a.int("limite").unwrap_or(30) as usize))) })),
        c(tool("contexte_lire", "Lire une entité du contexte",
            "Contenu complet d’une entité (par référence renvoyée par contexte_chercher / contexte_recents, ou par clé : « PAY-123 », « grp/app!42 », « @amartin »).")
            .input(vec![("ref", z::string().min(1).describe("Référence ou clé de l’entité"))])
            .run(|app, a, _| async move {
                let r = a.string("ref");
                Ok(match app.context_store.read(&r) { Some(found) => ok(found), None => fail(format!("Aucune entité « {r} » dans le contexte.")) })
            })),
    ]
}
