//! Lignes de logs côté cœur : analyse, recherche, profils, export et liens de partage (les
//! mêmes règles que l'interface, pour le MCP et le téléchargement d'historique).

pub mod parser;
pub mod share;

use crate::config::Profile;
use parser::{FieldNames, Level, ParsedLine, parse_line};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// JSON compact (`JSON.stringify`).
pub fn json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Métadonnées d'un message Kafka.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct KafkaMeta {
    pub partition: i32,
    /// Offset (chaîne : entier 64 bits).
    pub offset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Une ligne brute : ligne de log (`ts` Kubernetes) ou message Kafka (`meta`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LogLine {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<KafkaMeta>,
}

/// Ligne analysée.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub ts: Option<String>,
    pub text: String,
    pub parsed: ParsedLine,
    pub meta: Option<KafkaMeta>,
    /// Texte parcouru par la recherche : la ligne, plus la clé et les headers Kafka.
    pub haystack: String,
    pub notice: bool,
    /// Assemblage : libellé de la source.
    pub source: Option<String>,
}

impl LogEntry {
    pub fn new(line: LogLine, fields: &FieldNames) -> Self {
        let parsed = parse_line(&line.text, line.ts.as_deref(), fields);
        let haystack = match &line.meta {
            Some(meta) => {
                let mut parts = vec![line.text.clone(), meta.key.clone().unwrap_or_default()];
                for (k, v) in meta.headers.iter().flatten() { parts.push(format!("{k}={v}")); }
                parts.join("\n")
            }
            None => line.text.clone(),
        };
        LogEntry { ts: line.ts, text: line.text, parsed, meta: line.meta, haystack, notice: false, source: None }
    }

    /// Clé de tri temporel : horodatage analysé, horodatage brut, sinon fin de liste.
    pub fn time_key(&self) -> i64 {
        if let Some(t) = self.parsed.time { return t.timestamp_millis(); }
        self.ts.as_deref().and_then(parser::to_date_str).map(|d| d.timestamp_millis()).unwrap_or(i64::MAX)
    }
}

// --- Recherche --------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    pub text: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub regex: bool,
}

/// Recherche texte (sensible ou non à la casse) ou expression régulière.
pub struct Matcher {
    re: fancy_regex::Regex,
}

impl Matcher {
    /// `None` si la recherche est vide ; erreur lisible si l'expression est invalide.
    pub fn build(options: &SearchOptions) -> anyhow::Result<Option<Matcher>> {
        if options.text.is_empty() { return Ok(None); }
        let source = if options.regex { options.text.clone() } else { fancy_regex::escape(&options.text).into_owned() };
        let re = fancy_regex::RegexBuilder::new(&source)
            .case_insensitive(!options.case_sensitive)
            .build()
            .map_err(|e| anyhow::anyhow!("Expression régulière invalide : {e}"))?;
        Ok(Some(Matcher { re }))
    }

    pub fn test(&self, text: &str) -> bool { self.re.is_match(text).unwrap_or(false) }

    /// Plages [début, fin[ (octets) des correspondances non vides (200 au plus).
    pub fn ranges(&self, text: &str) -> Vec<(usize, usize)> {
        let mut out = vec![];
        for m in self.re.find_iter(text).flatten() {
            if m.start() == m.end() { continue; }
            out.push((m.start(), m.end()));
            if out.len() > 200 { break; }
        }
        out
    }
}

// --- Profils ----------------------------------------------------------------------------------

/// Joker `*` : n'importe quel segment ; `**` : n'importe quoi, `/` compris.
pub fn glob_to_regex(glob: &str) -> regex::Regex {
    let mut out = String::from("^");
    let mut rest = glob;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("**") {
            out.push_str(".*");
            rest = r;
        } else if let Some(r) = rest.strip_prefix('*') {
            out.push_str("[^/]*");
            rest = r;
        } else {
            let c = rest.chars().next().unwrap();
            out.push_str(&regex::escape(&c.to_string()));
            rest = &rest[c.len_utf8()..];
        }
    }
    out.push('$');
    regex::Regex::new(&out).unwrap_or_else(|_| regex::Regex::new("^$").unwrap())
}

/// Réglages effectifs d'un workload, après fusion des profils qui lui correspondent.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedProfile {
    pub key: String,
    pub matches: Vec<String>,
    pub fields: FieldNames,
    pub columns: Vec<String>,
    pub expand: Vec<String>,
    pub wrap: Vec<String>,
    pub parse_nested: bool,
}

/// Fusionne les profils dont `match` correspond à la clé (le premier prime, listes cumulées).
pub fn resolve_profile(profiles: &[Profile], key: &str) -> ResolvedProfile {
    let mut r = ResolvedProfile { key: key.into(), ..Default::default() };
    let mut parse_nested = None;
    for p in profiles {
        if p.match_ != "*" && !glob_to_regex(&p.match_).is_match(key) { continue; }
        r.matches.push(p.match_.clone());
        if r.fields.time.is_none() { r.fields.time = p.fields.time.clone(); }
        if r.fields.level.is_none() { r.fields.level = p.fields.level.clone(); }
        if r.fields.message.is_none() { r.fields.message = p.fields.message.clone(); }
        for (list, from) in [(&mut r.columns, &p.columns), (&mut r.expand, &p.expand), (&mut r.wrap, &p.wrap)] {
            for c in from { if !list.contains(c) { list.push(c.clone()); } }
        }
        if parse_nested.is_none() { parse_nested = p.parse_nested; }
    }
    r.parse_nested = parse_nested.unwrap_or(false);
    r
}

// --- Export -----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Ndjson,
    Text,
}

impl ExportFormat {
    /// `.log` / `.txt` : texte brut ; sinon NDJSON.
    pub fn for_file(path: &str) -> Self {
        let lower = path.to_lowercase();
        if lower.ends_with(".log") || lower.ends_with(".txt") { ExportFormat::Text } else { ExportFormat::Ndjson }
    }
    pub fn as_str(self) -> &'static str {
        match self { ExportFormat::Ndjson => "ndjson", ExportFormat::Text => "text" }
    }
}

fn value_of(entry: &LogEntry) -> Value {
    match &entry.parsed.json {
        Some(j) => Value::Object(j.clone()),
        None => Value::String(entry.text.clone()),
    }
}

/// Une ligne exportée (voir l'interface : même format).
pub fn export_line(entry: &LogEntry, format: ExportFormat) -> String {
    let src = entry.source.as_deref();
    let meta = entry.meta.as_ref();
    if format == ExportFormat::Text {
        let prefix = src.map(|s| format!("[{s}] ")).unwrap_or_default();
        let Some(meta) = meta else { return format!("{prefix}{}", entry.text) };
        let key = meta.key.as_ref().map(|k| format!(" {k}")).unwrap_or_default();
        return format!("{prefix}{} p{}@{}{key} {}", entry.ts.as_deref().unwrap_or(""), meta.partition, meta.offset, entry.text);
    }
    let headers = |m: &KafkaMeta| serde_json::to_value(m.headers.clone().unwrap_or_default()).unwrap();
    let ts = |e: &LogEntry| e.ts.clone().map(Value::String);
    let with = |mut v: serde_json::Map<String, Value>, k: &str, value: Option<Value>| {
        if let Some(x) = value { v.insert(k.into(), x); }
        v
    };
    if let Some(src) = src {
        let obj = match meta {
            Some(m) => {
                let mut o = serde_json::Map::new();
                o.insert("source".into(), json!(src));
                let mut o = with(o, "timestamp", ts(entry));
                o.insert("partition".into(), json!(m.partition));
                o.insert("offset".into(), json!(m.offset));
                o.insert("key".into(), m.key.clone().map(Value::String).unwrap_or(Value::Null));
                o.insert("headers".into(), headers(m));
                o.insert("value".into(), value_of(entry));
                o
            }
            None => {
                let mut o = serde_json::Map::new();
                o.insert("source".into(), json!(src));
                let mut o = with(o, "time", ts(entry));
                o.insert("log".into(), value_of(entry));
                o
            }
        };
        return json_text(&Value::Object(obj));
    }
    if let Some(m) = meta {
        let mut o = with(serde_json::Map::new(), "timestamp", ts(entry));
        o.insert("partition".into(), json!(m.partition));
        o.insert("offset".into(), json!(m.offset));
        o.insert("key".into(), m.key.clone().map(Value::String).unwrap_or(Value::Null));
        o.insert("headers".into(), headers(m));
        o.insert("value".into(), value_of(entry));
        return json_text(&Value::Object(o));
    }
    if entry.parsed.json.is_some() { return entry.text.trim().to_string(); }
    let mut o = with(serde_json::Map::new(), "time", ts(entry));
    o.insert("message".into(), json!(entry.text));
    json_text(&Value::Object(o))
}

/// Contenu du fichier : une ligne par entrée (les événements d'Ocekour sont exclus).
pub fn export_content(entries: &[LogEntry], format: ExportFormat) -> String {
    let lines: Vec<String> = entries.iter().filter(|e| !e.notice).map(|e| export_line(e, format)).collect();
    if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" }
}

/// Nom de fichier proposé : cible + date locale, sans caractère interdit sous Windows.
pub fn export_file_name(name: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let clean: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 0x20 { '_' } else { c }).collect();
    format!("{clean}-{}", now.format("%Y%m%d-%H%M"))
}

/// Filtre niveaux + recherche (comme la liste de l'interface).
pub fn filter_entries(entries: Vec<LogEntry>, levels: Option<&[Level]>, matcher: Option<&Matcher>) -> Vec<LogEntry> {
    entries
        .into_iter()
        .filter(|e| levels.is_none_or(|l| l.contains(&e.parsed.level)) && matcher.is_none_or(|m| m.test(&e.haystack)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<LogEntry> {
        let f = FieldNames::default();
        let mut v = vec![
            LogEntry::new(LogLine { ts: Some("2026-09-24T10:00:00.000Z".into()), text: r#"{"level":"info","msg":"ok"}"#.into(), meta: None }, &f),
            LogEntry::new(LogLine { ts: Some("2026-09-24T10:00:01.000Z".into()), text: "texte simple".into(), meta: None }, &f),
            LogEntry::new(
                LogLine {
                    ts: Some("2026-09-24T10:00:02.000Z".into()),
                    text: r#"{"id":7}"#.into(),
                    meta: Some(KafkaMeta { partition: 1, offset: "42".into(), key: Some("k".into()), headers: Some([("h".into(), "v".into())].into()), format: "json".into(), warning: None }),
                },
                &f,
            ),
        ];
        let mut notice = LogEntry::new(LogLine { ts: None, text: "Reconnexion".into(), meta: None }, &f);
        notice.notice = true;
        v.push(notice);
        v
    }

    #[test]
    fn export_ndjson() {
        let e = entries();
        assert_eq!(export_line(&e[0], ExportFormat::Ndjson), r#"{"level":"info","msg":"ok"}"#);
        assert_eq!(export_line(&e[1], ExportFormat::Ndjson), r#"{"time":"2026-09-24T10:00:01.000Z","message":"texte simple"}"#);
        assert_eq!(
            export_line(&e[2], ExportFormat::Ndjson),
            r#"{"timestamp":"2026-09-24T10:00:02.000Z","partition":1,"offset":"42","key":"k","headers":{"h":"v"},"value":{"id":7}}"#
        );
        assert_eq!(
            export_content(&e, ExportFormat::Text),
            "{\"level\":\"info\",\"msg\":\"ok\"}\ntexte simple\n2026-09-24T10:00:02.000Z p1@42 k {\"id\":7}\n"
        );
    }

    #[test]
    fn export_assemblage() {
        let mut e = LogEntry::new(LogLine { ts: Some("2026-09-24T10:00:00.000Z".into()), text: "bonjour".into(), meta: None }, &FieldNames::default());
        e.source = Some("api".into());
        assert_eq!(export_line(&e, ExportFormat::Text), "[api] bonjour");
        assert_eq!(export_line(&e, ExportFormat::Ndjson), r#"{"source":"api","time":"2026-09-24T10:00:00.000Z","log":"bonjour"}"#);
    }

    #[test]
    fn nom_de_fichier() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 9, 24, 9, 5, 0).unwrap();
        assert_eq!(export_file_name("api/app: 1", now), "api_app_ 1-20260924-0905");
        assert_eq!(ExportFormat::for_file("x.LOG"), ExportFormat::Text);
        assert_eq!(ExportFormat::for_file("x.jsonl"), ExportFormat::Ndjson);
    }

    #[test]
    fn recherche() {
        let m = Matcher::build(&SearchOptions { text: "a.b".into(), ..Default::default() }).unwrap().unwrap();
        assert!(m.test("xx A.B yy"));
        assert!(!m.test("axb"));
        let m = Matcher::build(&SearchOptions { text: "Error".into(), case_sensitive: true, regex: false }).unwrap().unwrap();
        assert!(!m.test("error"));
        let m = Matcher::build(&SearchOptions { text: r"commande \d+".into(), case_sensitive: false, regex: true }).unwrap().unwrap();
        assert_eq!(m.ranges("Commande 12 et commande 345"), vec![(0, 11), (15, 27)]);
        let e = Matcher::build(&SearchOptions { text: "(".into(), case_sensitive: false, regex: true }).err().unwrap();
        assert!(e.to_string().contains("Expression régulière invalide"));
        assert!(Matcher::build(&SearchOptions::default()).unwrap().is_none());
        let m = Matcher::build(&SearchOptions { text: "x*".into(), case_sensitive: false, regex: true }).unwrap().unwrap();
        assert_eq!(m.ranges("abxxc"), vec![(2, 4)]);
        // Syntaxe JavaScript : assertions avant / arrière.
        let m = Matcher::build(&SearchOptions { text: "(?<=id=)\\d+".into(), case_sensitive: false, regex: true }).unwrap().unwrap();
        assert_eq!(m.ranges("id=42"), vec![(3, 5)]);
    }

    fn profile(m: &str, f: impl FnOnce(&mut Profile)) -> Profile {
        let mut p = Profile { match_: m.into(), fields: Default::default(), columns: vec![], expand: vec![], wrap: vec![], parse_nested: None };
        f(&mut p);
        p
    }

    #[test]
    fn profils() {
        assert!(glob_to_regex("prod/*/api-*").is_match("prod/app-commandes/api-commandes"));
        assert!(!glob_to_regex("prod/*").is_match("prod/app/api"));
        assert!(glob_to_regex("prod/**").is_match("prod/app/api"));
        assert!(!glob_to_regex("recette.v2/*/*").is_match("recetteXv2/a/b"));
        let r = resolve_profile(
            &[
                profile("prod/app-commandes/api-commandes", |p| {
                    p.expand = vec!["stack_trace".into()];
                    p.fields.message = Some("msg".into());
                    p.wrap = vec!["@message".into()];
                }),
                profile("prod/*/*", |p| {
                    p.columns = vec!["traceId".into()];
                    p.parse_nested = Some(true);
                    p.expand = vec!["stack_trace".into(), "sql".into()];
                    p.wrap = vec!["@message".into(), "context".into()];
                }),
                profile("recette/*/*", |p| p.columns = vec!["ignoré".into()]),
                profile("*", |p| {
                    p.fields = FieldNames { message: Some("autre".into()), level: Some("sev".into()), time: None };
                    p.parse_nested = Some(false);
                }),
            ],
            "prod/app-commandes/api-commandes",
        );
        assert_eq!(r.matches, vec!["prod/app-commandes/api-commandes", "prod/*/*", "*"]);
        assert_eq!(r.fields, FieldNames { message: Some("msg".into()), level: Some("sev".into()), time: None });
        assert_eq!(r.columns, vec!["traceId"]);
        assert_eq!(r.expand, vec!["stack_trace", "sql"]);
        assert_eq!(r.wrap, vec!["@message", "context"]);
        assert!(r.parse_nested);
        assert!(!resolve_profile(&[], "a/b/c").parse_nested);
    }
}
