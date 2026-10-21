//! Découpage d'un flux d'octets en lignes et horodatages Kubernetes.

use crate::logs::LogLine;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

/// Au-delà, une ligne sans fin est coupée (protection mémoire).
const MAX_LINE: usize = 5_000_000;

/// Découpe un flux d'octets en lignes : morceaux coupés au milieu d'une ligne ou d'un
/// caractère UTF-8, fins de ligne `\n` ou `\r\n`.
#[derive(Default)]
pub struct LineSplitter {
    rest: Vec<u8>,
}

fn strip_cr(mut line: Vec<u8>) -> String {
    if line.last() == Some(&b'\r') { line.pop(); }
    String::from_utf8(line).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

impl LineSplitter {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = vec![];
        let mut start = 0;
        for (i, b) in chunk.iter().enumerate() {
            if *b == b'\n' {
                self.rest.extend_from_slice(&chunk[start..i]);
                lines.push(strip_cr(std::mem::take(&mut self.rest)));
                start = i + 1;
            }
        }
        self.rest.extend_from_slice(&chunk[start..]);
        if self.rest.len() > MAX_LINE {
            // Coupe à une frontière de caractère.
            let mut cut = self.rest.len();
            while cut > 0 && (self.rest[cut - 1] & 0xC0) == 0x80 { cut -= 1; }
            if cut > 0 && self.rest[cut - 1] >= 0xC0 { cut -= 1; }
            let tail = self.rest.split_off(cut);
            lines.push(strip_cr(std::mem::replace(&mut self.rest, tail)));
        }
        lines
    }

    /// Fin du flux : la dernière ligne incomplète éventuelle.
    pub fn flush(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.rest);
        if rest.is_empty() { vec![] } else { vec![strip_cr(rest)] }
    }
}

static K8S_TS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)) ").unwrap());

/// Sépare l'horodatage ajouté par Kubernetes (`timestamps=true`) du texte de la ligne.
pub fn parse_k8s_line(line: &str) -> LogLine {
    match K8S_TS.captures(line) {
        Some(m) => LogLine { ts: Some(m[1].to_string()), text: line[m[0].len()..].to_string(), meta: None },
        None => LogLine { ts: None, text: line.to_string(), meta: None },
    }
}

/// Clé de tri d'un horodatage RFC3339Nano (fractions complétées à 9 chiffres).
pub fn ts_key(ts: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(.*T\d\d:\d\d:\d\d)(?:\.(\d+))?Z$").unwrap());
    match RE.captures(ts) {
        Some(m) => format!("{}.{:0<9}Z", &m[1], m.get(2).map(|f| f.as_str()).unwrap_or("")),
        None => ts.to_string(),
    }
}

/// Écarte les lignes déjà reçues lors d'une reprise avec `sinceTime` (Kubernetes renvoie les
/// lignes de la seconde de reprise).
#[derive(Default)]
pub struct Deduplicator {
    last_key: String,
    seen_at_last: HashSet<String>,
    /// Horodatage de la dernière ligne acceptée.
    pub last: Option<String>,
}

impl Deduplicator {
    pub fn accept(&mut self, line: &LogLine) -> bool {
        let Some(ts) = &line.ts else { return true };
        let key = ts_key(ts);
        if key < self.last_key { return false; }
        if key == self.last_key {
            return self.seen_at_last.insert(line.text.clone());
        }
        self.last_key = key;
        self.last = Some(ts.clone());
        self.seen_at_last = HashSet::from([line.text.clone()]);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recolle_lignes_et_utf8() {
        let mut s = LineSplitter::default();
        let bytes = "première ligne\r\ndeuxième".as_bytes();
        let cut = "première ligne\r\ndeux".len() + 1; // au milieu du « è »
        assert!(s.push(&bytes[..3]).is_empty());
        assert_eq!(s.push(&bytes[3..cut]), vec!["première ligne"]);
        s.push(&bytes[cut..]);
        assert_eq!(s.push(b" ligne\n"), vec!["deuxième ligne"]);
        assert!(s.flush().is_empty());
        let mut s = LineSplitter::default();
        s.push(b"a\nb");
        assert_eq!(s.flush(), vec!["b"]);
    }

    #[test]
    fn horodatages() {
        let l = parse_k8s_line(r#"2026-09-24T10:00:00.123456789Z {"a":1}"#);
        assert_eq!((l.ts.as_deref(), l.text.as_str()), (Some("2026-09-24T10:00:00.123456789Z"), r#"{"a":1}"#));
        assert_eq!(parse_k8s_line("texte sans horodatage").ts, None);
        assert!(ts_key("2026-09-24T10:00:00.12345Z") > ts_key("2026-09-24T10:00:00.1234Z"));
        assert!(ts_key("2026-09-24T10:00:01Z") > ts_key("2026-09-24T10:00:00.9Z"));
    }

    #[test]
    fn deduplication() {
        let mut d = Deduplicator::default();
        let l = |ts: &str, text: &str| LogLine { ts: Some(format!("2026-09-24T10:00:{ts}Z")), text: text.into(), meta: None };
        assert!(d.accept(&l("01.1", "a")));
        assert!(d.accept(&l("02", "b")));
        assert!(d.accept(&l("02", "c")));
        assert_eq!(d.last.as_deref(), Some("2026-09-24T10:00:02Z"));
        assert!(!d.accept(&l("01.1", "a")));
        assert!(!d.accept(&l("02", "b")));
        assert!(!d.accept(&l("02", "c")));
        assert!(d.accept(&l("03", "d")));
    }
}
