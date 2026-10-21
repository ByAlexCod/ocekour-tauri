//! Petits utilitaires de texte et de dates partagés.

use chrono::{DateTime, SecondsFormat, Utc};
use std::cmp::Ordering;
use unicode_normalization::UnicodeNormalization;

/// Sans accents, en minuscules (recherche et tri insensibles).
pub fn fold(text: &str) -> String {
    text.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).collect::<String>().to_lowercase()
}

/// Ordre « alphabétique » à la française (casse et accents secondaires), comme `localeCompare`.
pub fn locale_cmp(a: &str, b: &str) -> Ordering {
    fold(a).cmp(&fold(b)).then_with(|| {
        // Même lettres : minuscule avant majuscule, sans accent avant accentué.
        let rank = |s: &str| s.chars().map(|c| (c.is_uppercase(), c as u32)).collect::<Vec<_>>();
        rank(a).cmp(&rank(b))
    })
}

/// Horodatage au format `toISOString()` (millisecondes, `Z`).
pub fn iso(date: DateTime<Utc>) -> String {
    date.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_iso() -> String { iso(Utc::now()) }

/// Date RFC 3339 quelconque → `toISOString()`.
pub fn to_iso(text: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(text).ok().map(|d| iso(d.with_timezone(&Utc)))
}

/// Millisecondes depuis l'époque d'une date ISO (`Date.parse`).
pub fn parse_ms(text: &str) -> Option<i64> {
    crate::logs::parser::to_date_str(text).map(|d| d.timestamp_millis())
}

pub fn now_ms() -> i64 { Utc::now().timestamp_millis() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tri() {
        let mut v = vec!["kube-system", "Default", "app-commandes", "éclair", "zèbre", "default"];
        v.sort_by(|a, b| locale_cmp(a, b));
        assert_eq!(v, vec!["app-commandes", "default", "Default", "éclair", "kube-system", "zèbre"]);
        assert_eq!(fold("Éléphant"), "elephant");
    }

    #[test]
    fn dates() {
        assert_eq!(to_iso("2026-09-24T10:00:00Z").unwrap(), "2026-09-24T10:00:00.000Z");
        assert_eq!(to_iso("2026-09-24T12:00:00+02:00").unwrap(), "2026-09-24T10:00:00.000Z");
    }
}
