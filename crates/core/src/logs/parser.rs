//! Analyse d'une ligne de log : JSON ou texte, heure, niveau, message (mêmes règles que
//! l'interface, `ui/src/lib/logs/parser.ts`).

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Unknown,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Trace => "trace",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
            Level::Fatal => "fatal",
            Level::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> Option<Level> {
        Some(match s {
            "trace" => Level::Trace,
            "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" => Level::Warn,
            "error" => Level::Error,
            "fatal" => Level::Fatal,
            "unknown" => Level::Unknown,
            _ => return None,
        })
    }
}

/// Noms des champs clés ; ceux du profil priment sur la détection.
pub use crate::config::Fields as FieldNames;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLine {
    /// Objet JSON si la ligne entière en est un.
    pub json: Option<Map<String, Value>>,
    pub time: Option<DateTime<Utc>>,
    pub level: Level,
    /// Message sur une ligne pour la liste.
    pub message: String,
}

const TIME: [&str; 7] = ["@timestamp", "timestamp", "time", "ts", "date", "datetime", "@t"];
const LEVEL: [&str; 8] = ["level", "severity", "log.level", "lvl", "loglevel", "levelname", "@l", "log_level"];
const MESSAGE: [&str; 7] = ["message", "msg", "@m", "@mt", "log", "text", "event"];

/// Au-delà, la ligne n'est pas analysée comme JSON.
const MAX_JSON: usize = 1_000_000;

static ANSI: Lazy<Regex> = Lazy::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap());

pub fn strip_ansi(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\x1b') { ANSI.replace_all(text, "") } else { std::borrow::Cow::Borrowed(text) }
}

/// Lit `a.b.c` en objet imbriqué, ou la clé plate `"a.b.c"` si elle existe.
pub fn get_path<'a>(obj: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    if let Some(v) = obj.get(path) { return Some(v); }
    if !path.contains('.') { return None; }
    let mut current: Option<&Value> = None;
    for (i, key) in path.split('.').enumerate() {
        let map = if i == 0 { Some(obj) } else { current?.as_object() };
        current = map?.get(key);
    }
    current
}

/// Comme `get_path`, mais traverse aussi les chaînes qui contiennent du JSON.
pub fn get_field_deep(obj: &Map<String, Value>, path: &str) -> Option<Value> {
    if let Some(v) = get_path(obj, path) { return Some(v.clone()); }
    let mut current = Value::Object(obj.clone());
    for key in path.split('.') {
        if let Value::String(s) = &current { current = expand_nested(&Value::String(s.clone()), 1); }
        current = current.as_object()?.get(key)?.clone();
    }
    Some(current)
}

fn pick(obj: &Map<String, Value>, forced: Option<&str>, candidates: &[&str]) -> Option<Value> {
    if let Some(f) = forced { return get_field_deep(obj, f); }
    for key in candidates {
        if let Some(v) = get_path(obj, key) {
            if !v.is_null() && v.as_str() != Some("") { return Some(v.clone()); }
        }
    }
    None
}

pub fn parse_line(text: &str, k8s_ts: Option<&str>, fields: &FieldNames) -> ParsedLine {
    let clean = strip_ansi(text);
    if let Some(json) = parse_json_object(&clean) {
        let message = pick(&json, fields.message.as_deref(), &MESSAGE);
        let time = pick(&json, fields.time.as_deref(), &TIME).and_then(|v| to_date(&v)).or_else(|| k8s_ts.and_then(to_date_str));
        let level = normalize_level(pick(&json, fields.level.as_deref(), &LEVEL).as_ref());
        let message = one_line(&match message {
            None => clean.to_string(),
            Some(v) => stringify(&v),
        }, 2_000);
        return ParsedLine { json: Some(json), time, level, message };
    }
    ParsedLine { json: None, time: k8s_ts.and_then(to_date_str), level: text_level(&clean), message: one_line(&clean, 2_000) }
}

/// Valeur brute du message d'une ligne JSON.
pub fn raw_message(json: &Map<String, Value>, fields: &FieldNames) -> Option<Value> {
    pick(json, fields.message.as_deref(), &MESSAGE)
}

fn parse_json_object(text: &str) -> Option<Map<String, Value>> {
    let trimmed = text.trim_start_matches(|c: char| (c as u32) <= 32);
    if !trimmed.starts_with('{') || text.len() > MAX_JSON { return None; }
    let trimmed = trimmed.trim_end_matches(|c: char| (c as u32) <= 32);
    if !trimmed.ends_with('}') { return None; }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// Texte d'une valeur : chaîne telle quelle, sinon JSON.
pub fn stringify(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => crate::logs::json_text(other),
    }
}

/// Liste compacte : retours à la ligne visibles, longueur bornée (caractères).
pub fn one_line(text: &str, max: usize) -> String {
    let short: String = if text.chars().nth(max).is_some() {
        let mut s: String = text.chars().take(max).collect();
        s.push('…');
        s
    } else {
        text.to_string()
    };
    if !short.contains('\n') && !short.contains('\t') { return short; }
    short.replace("\r\n", " ⏎ ").replace('\n', " ⏎ ").replace('\t', " ")
}

fn level_word(word: &str) -> Option<Level> {
    Some(match word {
        "trace" | "verbose" | "vrb" | "finest" | "finer" => Level::Trace,
        "debug" | "dbg" | "fine" | "d" => Level::Debug,
        "info" | "information" | "informational" | "notice" | "inf" | "i" => Level::Info,
        "warn" | "warning" | "wrn" | "w" => Level::Warn,
        "error" | "err" | "severe" | "e" => Level::Error,
        "fatal" | "critical" | "crit" | "panic" | "emerg" | "emergency" | "alert" | "ftl" | "f" => Level::Fatal,
        _ => return None,
    })
}

fn numeric_level(n: f64) -> Level {
    if n >= 10.0 {
        return if n >= 60.0 {
            Level::Fatal
        } else if n >= 50.0 {
            Level::Error
        } else if n >= 40.0 {
            Level::Warn
        } else if n >= 30.0 {
            Level::Info
        } else if n >= 20.0 {
            Level::Debug
        } else {
            Level::Trace
        };
    }
    // syslog : 0 emerg … 7 debug
    match n as i64 {
        _ if n.fract() != 0.0 || n < 0.0 => Level::Unknown,
        0..=2 => Level::Fatal,
        3 => Level::Error,
        4 => Level::Warn,
        5 | 6 => Level::Info,
        7 => Level::Debug,
        _ => Level::Unknown,
    }
}

/// Niveaux texte (WARN, warning, E…) et numériques (pino 10–60, syslog 0–7).
pub fn normalize_level(value: Option<&Value>) -> Level {
    match value {
        Some(Value::Number(n)) => numeric_level(n.as_f64().unwrap_or(-1.0)),
        Some(Value::String(s)) => {
            if let Some(l) = level_word(s).or_else(|| level_word(&s.to_lowercase())) { return l; }
            let word = s.trim().to_lowercase();
            if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                return word.parse::<f64>().map(numeric_level).unwrap_or(Level::Unknown);
            }
            level_word(&word).unwrap_or(Level::Unknown)
        }
        _ => Level::Unknown,
    }
}

static TEXT_LEVEL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(TRACE|DEBUG|INFO|NOTICE|WARN(?:ING)?|ERROR|SEVERE|FATAL|CRITICAL|PANIC)\b").unwrap());
static LOGFMT_LEVEL: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)\b(?:level|lvl|severity)=["']?(\w+)"#).unwrap());

/// Niveau d'une ligne texte : mot-clé en majuscules ou `level=…` (logfmt).
fn text_level(text: &str) -> Level {
    let head: String = text.chars().take(300).collect();
    if let Some(m) = LOGFMT_LEVEL.captures(&head) {
        return normalize_level(Some(&Value::String(m[1].to_string())));
    }
    match TEXT_LEVEL.captures(&head) {
        Some(m) => normalize_level(Some(&Value::String(m[1].to_string()))),
        None => Level::Unknown,
    }
}

static EPOCH: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{9,19}(\.\d+)?$").unwrap());

fn from_epoch(n: f64) -> Option<DateTime<Utc>> {
    let ms = if n < 1e11 { n * 1_000.0 } else if n < 1e14 { n } else if n < 1e17 { n / 1_000.0 } else { n / 1_000_000.0 };
    if !ms.is_finite() { return None; }
    DateTime::from_timestamp_millis(ms.trunc() as i64)
}

/// ISO 8601, formats Logback (`2026-09-24 10:00:00,123`), epoch s / ms / µs / ns.
pub fn to_date(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Number(n) => from_epoch(n.as_f64()?),
        Value::String(s) => to_date_str(s),
        _ => None,
    }
}

static LOGBACK_T: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(\d{4}-\d\d-\d\d) ").unwrap());
static COMMA_MS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(:\d\d),(\d+)").unwrap());
static ISO: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^([+-]\d{6}|\d{4})-(\d\d)-(\d\d)(?:T(\d\d):(\d\d)(?::(\d\d)(?:\.(\d+))?)?)?\s*(Z|z|[+-]\d\d(?::?\d\d)?)?$").unwrap()
});

pub fn to_date_str(value: &str) -> Option<DateTime<Utc>> {
    if value.is_empty() { return None; }
    if EPOCH.is_match(value) { return from_epoch(value.parse().ok()?); }
    let normalized = LOGBACK_T.replace(value.trim(), "${1}T");
    let normalized = COMMA_MS.replace(&normalized, "$1.$2");
    if let Some(d) = parse_iso(&normalized) { return Some(d); }
    // Autres formats courants (RFC 2822, « 2026/09/24 10:00:00 »).
    if let Ok(d) = DateTime::parse_from_rfc2822(&normalized) { return Some(d.with_timezone(&Utc)); }
    for f in ["%Y/%m/%d %H:%M:%S", "%Y/%m/%d %H:%M:%S%.f", "%d/%m/%Y %H:%M:%S", "%b %d %Y %H:%M:%S", "%a %b %d %Y %H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(&normalized, f) { return local(n); }
    }
    None
}

fn local(n: NaiveDateTime) -> Option<DateTime<Utc>> {
    Local.from_local_datetime(&n).earliest().map(|d| d.with_timezone(&Utc))
}

/// ISO 8601 comme `Date.parse` : sans fuseau, une date seule est en UTC et une date-heure en
/// heure locale ; fractions de seconde au-delà de la milliseconde ignorées.
fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    let c = ISO.captures(s)?;
    let num = |i: usize| c.get(i).map(|m| m.as_str().parse::<i64>().ok()).unwrap_or(Some(0));
    let date = NaiveDate::from_ymd_opt(num(1)? as i32, num(2)? as u32, num(3)? as u32)?;
    let has_time = c.get(4).is_some();
    let ms = c.get(7).map(|m| {
        let digits: String = m.as_str().chars().take(3).collect();
        format!("{digits:0<3}").parse::<u32>().unwrap_or(0)
    }).unwrap_or(0);
    let naive = date.and_hms_milli_opt(num(4)? as u32, num(5)? as u32, num(6)? as u32, ms)?;
    match c.get(8).map(|m| m.as_str()) {
        Some("Z" | "z") => Some(Utc.from_utc_datetime(&naive)),
        Some(offset) => {
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let digits: String = offset[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let h: i64 = digits.get(..2)?.parse().ok()?;
            let m: i64 = digits.get(2..4).unwrap_or("0").parse().ok()?;
            let secs = sign * (h * 3600 + m * 60);
            Some(Utc.from_utc_datetime(&(naive - chrono::Duration::seconds(secs))))
        }
        None if has_time => local(naive),
        None => Some(Utc.from_utc_datetime(&naive)),
    }
}

/// Désérialise les chaînes qui contiennent du JSON, jusqu'à `max_depth` niveaux (copie).
pub fn expand_nested(value: &Value, max_depth: u32) -> Value {
    if max_depth == 0 { return value.clone(); }
    match value {
        Value::String(s) => {
            let t = s.trim();
            if t.len() < MAX_JSON && ((t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'))) {
                if let Ok(v) = serde_json::from_str::<Value>(t) { return expand_nested(&v, max_depth - 1); }
            }
            value.clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| expand_nested(v, max_depth)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), expand_nested(v, max_depth))).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const K8S: &str = "2026-09-24T08:00:00.5Z";
    fn none() -> FieldNames { FieldNames::default() }

    #[test]
    fn logback_json() {
        let text = json!({ "@timestamp": "2026-09-24T10:00:00.123+02:00", "level": "WARN", "logger_name": "fr.acme.X", "message": "Stock bas", "stack_trace": "a\nb" }).to_string();
        let p = parse_line(&text, Some(K8S), &none());
        assert_eq!(p.level, Level::Warn);
        assert_eq!(p.message, "Stock bas");
        assert_eq!(crate::text::iso(p.time.unwrap()), "2026-09-24T08:00:00.123Z");
        assert_eq!(p.json.unwrap()["logger_name"], json!("fr.acme.X"));
    }

    #[test]
    fn pino() {
        let p = parse_line(r#"{"level":50,"time":1790245591136,"msg":"Échec"}"#, None, &none());
        assert_eq!((p.level, p.message.as_str()), (Level::Error, "Échec"));
        assert_eq!(p.time.unwrap().timestamp_millis(), 1790245591136);
    }

    #[test]
    fn serilog_et_imbrique() {
        let p = parse_line(r#"{"@t":"2026-09-24T10:00:00Z","@l":"Error","@mt":"Boom"}"#, None, &none());
        assert_eq!((p.level, p.message.as_str()), (Level::Error, "Boom"));
        assert_eq!(parse_line(r#"{"log":{"level":"debug"},"message":"x"}"#, None, &none()).level, Level::Debug);
    }

    #[test]
    fn champs_imposes() {
        let f = FieldNames { level: Some("sev".into()), message: Some("txt".into()), time: Some("at".into()) };
        let p = parse_line(r#"{"sev":"E","txt":"custom","at":"2026-01-01T00:00:00Z"}"#, None, &f);
        assert_eq!((p.level, p.message.as_str()), (Level::Error, "custom"));
        assert_eq!(crate::text::iso(p.time.unwrap()), "2026-01-01T00:00:00.000Z");
    }

    #[test]
    fn json_sans_message() {
        let p = parse_line(r#"{"a":1}"#, Some(K8S), &none());
        assert_eq!(p.message, r#"{"a":1}"#);
        assert_eq!(crate::text::iso(p.time.unwrap()), "2026-09-24T08:00:00.500Z");
        assert_eq!(p.level, Level::Unknown);
    }

    #[test]
    fn texte() {
        assert_eq!(parse_line("2026-09-24 10:00:00 ERROR [main] Boom", None, &none()).level, Level::Error);
        assert_eq!(parse_line("time=… level=warning msg=\"disk\"", None, &none()).level, Level::Warn);
        let p = parse_line("\x1b[32mINFO\x1b[0m démarré", None, &none());
        assert_eq!((p.level, p.message.as_str()), (Level::Info, "INFO démarré"));
        assert_eq!(parse_line("[2026] \"GET /api HTTP/1.1\" 200", None, &none()).level, Level::Unknown);
        assert!(parse_line(r#"{"a":"#, None, &none()).json.is_none());
        assert!(parse_line("[1,2]", None, &none()).json.is_none());
    }

    #[test]
    fn niveaux() {
        for (input, expected) in [
            (json!("WARNING"), Level::Warn),
            (json!("W"), Level::Warn),
            (json!("Information"), Level::Info),
            (json!("CRITICAL"), Level::Fatal),
            (json!(30), Level::Info),
            (json!(60), Level::Fatal),
            (json!(3), Level::Error),
            (json!(7), Level::Debug),
            (json!("40"), Level::Warn),
            (json!("banane"), Level::Unknown),
        ] {
            assert_eq!(normalize_level(Some(&input)), expected, "{input}");
        }
        assert_eq!(normalize_level(None), Level::Unknown);
    }

    #[test]
    fn dates() {
        let d = to_date_str("2026-09-24 10:00:00,123").unwrap();
        assert_eq!(d.with_timezone(&Local).timestamp_subsec_millis(), 123);
        assert_eq!(to_date(&json!(1790245591)).unwrap().timestamp_millis(), 1790245591000);
        assert_eq!(to_date(&json!(1790245591136u64)).unwrap().timestamp_millis(), 1790245591136);
        assert_eq!(to_date_str("1790245591136000000").unwrap().timestamp_millis(), 1790245591136);
        assert!(to_date_str("pas une date").is_none());
        assert_eq!(crate::text::iso(to_date_str("2026-09-24T10:00:00.123456789Z").unwrap()), "2026-09-24T10:00:00.123Z");
        assert_eq!(crate::text::iso(to_date_str("2026-09-24").unwrap()), "2026-09-24T00:00:00.000Z");
    }

    #[test]
    fn utilitaires() {
        assert_eq!(one_line("a\nb\tc", 2_000), "a ⏎ b c");
        assert_eq!(strip_ansi("\x1b[1;31mrouge\x1b[0m"), "rouge");
        let input = json!({ "payload": "{\"client\":{\"id\":1},\"lignes\":\"[1,2]\"}", "texte": "{pas du json" });
        assert_eq!(expand_nested(&input, 3), json!({ "payload": { "client": { "id": 1 }, "lignes": [1, 2] }, "texte": "{pas du json" }));
        let obj = json!({ "payload": "{\"client\":{\"id\":7}}", "a.b": 1, "c": { "d": 2 } });
        let obj = obj.as_object().unwrap();
        assert_eq!(get_field_deep(obj, "payload.client.id"), Some(json!(7)));
        assert_eq!(get_field_deep(obj, "a.b"), Some(json!(1)));
        assert_eq!(get_field_deep(obj, "c.d"), Some(json!(2)));
        assert_eq!(get_field_deep(obj, "payload.absent"), None);
    }
}
