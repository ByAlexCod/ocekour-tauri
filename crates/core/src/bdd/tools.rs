//! Outils MCP « bases de données » (LECTURE SEULE). Visibles seulement si au moins une base
//! est configurée. Toute requête passe par la garde lecture seule + une transaction read-only.

use crate::mcp::{Args, Tool, Visibility, ok, tool};
use crate::schema::{self as z};
use serde_json::json;

fn quote(engine: &str, parts: &[String]) -> String {
    if engine == "sqlserver" {
        parts.iter().map(|p| format!("[{}]", p.replace(']', "]]"))).collect::<Vec<_>>().join(".")
    } else {
        parts.iter().map(|p| format!("\"{}\"", p.replace('"', "\"\""))).collect::<Vec<_>>().join(".")
    }
}

/// SELECT borné d'une table (syntaxe du moteur).
pub fn preview_sql(engine: &str, table: &str, schema: Option<&str>, n: i64) -> String {
    let parts: Vec<String> = match schema { Some(s) => vec![s.into(), table.into()], None => vec![table.into()] };
    let q = quote(engine, &parts);
    match engine {
        "postgres" => format!("SELECT * FROM {q} LIMIT {n}"),
        "oracle" => format!("SELECT * FROM {q} FETCH FIRST {n} ROWS ONLY"),
        _ => format!("SELECT TOP {n} * FROM {q}"),
    }
}

pub fn tools() -> Vec<Tool> {
    let b = |t: Tool| t.visible(Visibility::Bdd).read_only();
    let id_s = || z::string().min(1).describe("Id de la base");
    vec![
        b(tool("bdd_connexions", "Lister les bases de données",
            "Bases configurées (PostgreSQL, SQL Server, Oracle) consultables en lecture seule. Renvoie id, moteur, hôte, base, utilisateur.")
            .run(|app, _, _| async move { Ok(ok(json!({ "bases": app.databases.list() }))) })),
        b(tool("bdd_tables", "Lister tables et vues", "Schémas, tables et vues de la base (hors objets système).")
            .input(vec![("connexion", z::string().min(1).describe("Id de la base (voir bdd_connexions)"))])
            .run(|app, a, _| async move { Ok(ok(json!({ "tables": app.databases.tables(&a.string("connexion")).await? }))) })),
        b(tool("bdd_colonnes", "Colonnes d’une table", "Colonnes (nom, type, nullable) d’une table ou vue.")
            .input(vec![
                ("connexion", id_s()),
                ("table", z::string().min(1).describe("Nom de la table/vue")),
                ("schema", z::string().optional().describe("Schéma / propriétaire (facultatif)")),
            ])
            .run(|app, a, _| async move {
                Ok(ok(json!({ "colonnes": app.databases.columns(&a.string("connexion"), &a.string("table"), a.str("schema").as_deref()).await? })))
            })),
        b(tool("bdd_apercu", "Aperçu d’une table", "Premières lignes d’une table ou vue (SELECT * borné).")
            .input(vec![
                ("connexion", id_s()),
                ("table", z::string().min(1).describe("Nom de la table/vue")),
                ("schema", z::string().optional().describe("Schéma / propriétaire (facultatif)")),
                ("limite", z::int().min(1).max(1_000).optional().describe("Lignes au plus (défaut : 50)")),
            ])
            .run(|app, a, _| async move {
                let id = a.string("connexion");
                let conn = app.databases.get(&id)?;
                let n = a.int("limite").unwrap_or(50);
                let sql = preview_sql(&conn.engine, &a.string("table"), a.str("schema").as_deref(), n);
                Ok(ok(app.databases.query(&id, &sql, n as usize).await?))
            })),
        b(tool("bdd_requete", "Requête SQL (lecture seule)",
            "Exécute UNE requête de lecture (SELECT / WITH / EXPLAIN). Toute écriture (INSERT, UPDATE, DELETE, DDL…) est refusée. Le résultat est borné en lignes.")
            .input(vec![
                ("connexion", id_s()),
                ("sql", z::string().min(1).describe("Requête de lecture (une seule instruction)")),
                ("limite", z::int().min(1).max(10_000).optional().describe("Lignes renvoyées au plus (défaut : 200)")),
            ])
            .run(|app, a, _| async move { Ok(ok(app.databases.query(&a.string("connexion"), &a.string("sql"), a.int("limite").unwrap_or(200) as usize).await?)) })),
        b(tool("bdd_expliquer", "Plan d’exécution (PostgreSQL)",
            "Plan d’exécution d’une requête SELECT (EXPLAIN). Pris en charge pour PostgreSQL ; pour les autres moteurs, préférez bdd_requete.")
            .input(vec![("connexion", id_s()), ("sql", z::string().min(1).describe("Requête SELECT à analyser"))])
            .run(|app, a, _| async move {
                let id = a.string("connexion");
                if app.databases.get(&id)?.engine != "postgres" {
                    return Ok(ok("EXPLAIN n’est pris en charge que pour PostgreSQL en lecture seule."));
                }
                Ok(ok(app.databases.query(&id, &format!("EXPLAIN {}", a.string("sql")), 1_000).await?))
            })),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apercu_par_moteur() {
        assert_eq!(preview_sql("postgres", "t", Some("public"), 5), "SELECT * FROM \"public\".\"t\" LIMIT 5");
        assert_eq!(preview_sql("oracle", "T\"x", None, 5), "SELECT * FROM \"T\"\"x\" FETCH FIRST 5 ROWS ONLY");
        assert_eq!(preview_sql("sqlserver", "a]b", Some("dbo"), 5), "SELECT TOP 5 * FROM [dbo].[a]]b]");
    }
}
