//! Garantie « lecture seule » au niveau SQL (défense en profondeur, en plus de la transaction
//! read-only côté moteur). Une seule instruction de lecture par appel.

use once_cell::sync::Lazy;
use regex::Regex;

/// Mots-clés autorisés en tête d'instruction.
const ALLOWED_START: [&str; 6] = ["SELECT", "WITH", "EXPLAIN", "SHOW", "DESC", "DESCRIBE"];

/// Mots-clés interdits n'importe où (écriture, DDL, transaction, procédures).
static FORBIDDEN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(INSERT|UPDATE|DELETE|MERGE|UPSERT|REPLACE|CREATE|DROP|ALTER|TRUNCATE|RENAME|GRANT|REVOKE|COMMIT|ROLLBACK|SAVEPOINT|LOCK|CALL|EXEC|EXECUTE|PREPARE|DECLARE|SET|INTO|VACUUM|ANALYZE|REINDEX|CLUSTER|COPY|BACKUP|RESTORE|ATTACH|DETACH|PRAGMA)\b").unwrap()
});

/// Retire les commentaires SQL et les littéraux de chaîne.
fn strip(sql: &str) -> String {
    static BLOCK: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)/\*.*?\*/").unwrap());
    static LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"--[^\n]*").unwrap());
    static SINGLE: Lazy<Regex> = Lazy::new(|| Regex::new(r"'(?:''|[^'])*'").unwrap());
    static DOUBLE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""(?:""|[^"])*""#).unwrap());
    let s = BLOCK.replace_all(sql, " ");
    let s = LINE.replace_all(&s, " ");
    let s = SINGLE.replace_all(&s, "''");
    DOUBLE.replace_all(&s, "\"\"").into_owned()
}

fn trim_semicolons(s: &str) -> &str { s.trim_end().trim_end_matches(|c: char| c == ';' || c.is_whitespace()) }

/// Motif de refus, ou `None` si la requête est en lecture seule.
pub fn read_only_issue(sql: &str) -> Option<String> {
    let raw = sql.trim();
    if raw.is_empty() { return Some("Requête vide.".into()); }
    let stripped = strip(raw);
    let cleaned = trim_semicolons(stripped.trim());
    if cleaned.is_empty() { return Some("Requête vide.".into()); }
    if cleaned.contains(';') { return Some("Une seule instruction est autorisée (point-virgule intermédiaire détecté).".into()); }
    static START: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\(*\s*([a-zA-Z]+)").unwrap());
    let start = START.captures(cleaned).map(|c| c[1].to_uppercase());
    if !start.as_deref().is_some_and(|s| ALLOWED_START.contains(&s)) {
        return Some(format!("Seules les lectures sont permises (début attendu : {}).", ALLOWED_START.join(", ")));
    }
    FORBIDDEN.captures(cleaned).map(|c| format!("Mot-clé interdit en lecture seule : {}.", c[1].to_uppercase()))
}

/// Requête sans `;` final, ou erreur.
pub fn assert_read_only(sql: &str) -> anyhow::Result<String> {
    if let Some(issue) = read_only_issue(sql) { anyhow::bail!(issue); }
    Ok(trim_semicolons(sql.trim()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lectures_acceptees() {
        for sql in ["SELECT * FROM clients", "  with t as (select 1) select * from t ", "EXPLAIN SELECT 1", "SHOW TABLES", "(SELECT 1)"] {
            assert_eq!(read_only_issue(sql), None, "{sql}");
        }
    }

    #[test]
    fn ecritures_refusees() {
        for sql in ["INSERT INTO t VALUES (1)", "UPDATE t SET a = 1", "DELETE FROM t", "DROP TABLE t", "CREATE TABLE t (a int)", "ALTER TABLE t ADD b int", "TRUNCATE t", "MERGE INTO t ...", "GRANT SELECT ON t TO x", "CALL proc()"] {
            assert!(read_only_issue(sql).is_some(), "{sql}");
        }
        assert!(read_only_issue("SELECT 1; INSERT INTO t VALUES (1)").unwrap().to_lowercase().contains("une seule instruction"));
        assert!(read_only_issue("SELECT * INTO backup FROM t").unwrap().contains("interdit"));
        assert!(read_only_issue("UPDATE t SET a=1; SELECT 1").is_some());
    }

    #[test]
    fn commentaires_et_chaines_ignores() {
        assert_eq!(read_only_issue("SELECT 'INSERT INTO' AS note"), None);
        assert_eq!(read_only_issue("SELECT 1 -- DROP TABLE t"), None);
        assert_eq!(read_only_issue("SELECT 1 /* UPDATE */ FROM dual"), None);
        assert!(read_only_issue("   ").is_some());
        assert!(read_only_issue("-- juste un commentaire").is_some());
    }

    #[test]
    fn point_virgule_final() {
        assert_eq!(assert_read_only("SELECT 1;").unwrap(), "SELECT 1");
        assert!(assert_read_only("DELETE FROM t").is_err());
    }
}
