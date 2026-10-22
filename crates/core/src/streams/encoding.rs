//! Encodages des fichiers de logs : UTF-8, UTF-16 (BOM), Latin-1, Windows-1252, et découpage
//! en lignes au niveau des octets (positions exactes, pour la reprise d'un flux).

use serde::{Deserialize, Serialize};

/// Au-delà, une ligne est coupée (protection mémoire).
pub const MAX_LINE_BYTES: usize = 1_000_000;
pub const TRUNCATED: &str = "… (tronqué)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-16le")]
    Utf16Le,
    #[serde(rename = "utf-16be")]
    Utf16Be,
    #[serde(rename = "latin-1")]
    Latin1,
    #[serde(rename = "windows-1252")]
    Windows1252,
}

impl Encoding {
    /// Nom saisi par l'utilisateur (`utf-16` = petit-boutiste, comme Windows).
    pub fn parse(name: &str) -> Option<Encoding> {
        match name.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "utf-8" | "utf8" => Some(Encoding::Utf8),
            "utf-16" | "utf16" | "utf-16le" | "utf-16-le" => Some(Encoding::Utf16Le),
            "utf-16be" | "utf-16-be" => Some(Encoding::Utf16Be),
            "latin-1" | "latin1" | "iso-8859-1" => Some(Encoding::Latin1),
            "windows-1252" | "cp1252" | "cp-1252" => Some(Encoding::Windows1252),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf-8",
            Encoding::Utf16Le => "utf-16le",
            Encoding::Utf16Be => "utf-16be",
            Encoding::Latin1 => "latin-1",
            Encoding::Windows1252 => "windows-1252",
        }
    }

    /// Taille d'une unité (fin de ligne alignée sur 2 octets en UTF-16).
    pub fn unit(self) -> usize { if matches!(self, Encoding::Utf16Le | Encoding::Utf16Be) { 2 } else { 1 } }

    /// Une fin de ligne (`\n`) commence à `bytes[i]` (`i` aligné sur l'unité).
    pub fn is_newline(self, bytes: &[u8], i: usize) -> bool {
        match self {
            Encoding::Utf16Le => bytes.get(i) == Some(&0x0A) && bytes.get(i + 1) == Some(&0),
            Encoding::Utf16Be => bytes.get(i) == Some(&0) && bytes.get(i + 1) == Some(&0x0A),
            _ => bytes.get(i) == Some(&b'\n'),
        }
    }

    /// Texte d'une ligne (sans sa fin de ligne), `\r` final retiré, octets invalides remplacés.
    pub fn decode(self, bytes: &[u8]) -> String {
        let mut text = match self {
            Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Encoding::Utf16Le => encoding_rs::UTF_16LE.decode_without_bom_handling(bytes).0.into_owned(),
            Encoding::Utf16Be => encoding_rs::UTF_16BE.decode_without_bom_handling(bytes).0.into_owned(),
            Encoding::Latin1 => bytes.iter().map(|b| *b as char).collect(),
            Encoding::Windows1252 => encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(),
        };
        if text.ends_with('\r') { text.pop(); }
        text
    }
}

/// Encodage reconnu par la marque d'ordre des octets, et taille de cette marque.
pub fn detect_bom(head: &[u8]) -> Option<(Encoding, u64)> {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) { return Some((Encoding::Utf8, 3)); }
    if head.starts_with(&[0xFF, 0xFE]) { return Some((Encoding::Utf16Le, 2)); }
    if head.starts_with(&[0xFE, 0xFF]) { return Some((Encoding::Utf16Be, 2)); }
    None
}

/// Une ligne lue : texte et position (octets) juste après sa fin de ligne.
#[derive(Debug, Clone, PartialEq)]
pub struct RawLine {
    pub text: String,
    pub end: u64,
}

/// Découpe des octets en lignes, en gardant la position de chaque fin de ligne.
pub struct ByteSplitter {
    encoding: Encoding,
    rest: Vec<u8>,
    /// Position dans le fichier du premier octet de `rest`.
    start: u64,
    /// Ligne trop longue déjà émise (tronquée) : on saute jusqu'à la prochaine fin de ligne.
    skipping: bool,
    /// Ligne incomplète déjà émise (fin de fichier) : sa fin de ligne, si elle arrive ensuite,
    /// ne doit pas produire une ligne vide.
    flushed: bool,
}

impl ByteSplitter {
    pub fn new(encoding: Encoding, start: u64) -> Self { ByteSplitter { encoding, rest: vec![], start, skipping: false, flushed: false } }

    /// Position juste après la dernière ligne complète.
    pub fn position(&self) -> u64 { self.start }

    /// Octets en attente (ligne en cours d'écriture).
    pub fn pending(&self) -> usize { self.rest.len() }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<RawLine> {
        self.rest.extend_from_slice(chunk);
        let unit = self.encoding.unit();
        let mut lines = vec![];
        let mut from = 0;
        let mut i = 0;
        while i + unit <= self.rest.len() {
            if std::mem::take(&mut self.flushed) && i == 0 && self.encoding.is_newline(&self.rest, 0) {
                from = unit;
                i = unit;
                continue;
            }
            if self.encoding.is_newline(&self.rest, i) {
                let end = self.start + (i + unit) as u64;
                if self.skipping {
                    // Fin de la ligne tronquée (déjà émise).
                    self.skipping = false;
                } else {
                    lines.push(RawLine { text: self.encoding.decode(&self.rest[from..i]), end });
                }
                from = i + unit;
            }
            i += unit;
        }
        self.start += from as u64;
        self.rest.drain(..from);
        if self.rest.len() > MAX_LINE_BYTES {
            let keep = MAX_LINE_BYTES - MAX_LINE_BYTES % unit;
            if !self.skipping {
                lines.push(RawLine { text: format!("{}{TRUNCATED}", self.encoding.decode(&self.rest[..keep])), end: self.start + self.rest.len() as u64 });
            }
            self.skipping = true;
            self.start += self.rest.len() as u64;
            self.rest.clear();
        }
        lines
    }

    /// Ligne incomplète en attente (fin de fichier atteinte).
    pub fn flush(&mut self) -> Option<RawLine> {
        if self.rest.is_empty() { return None; }
        let end = self.start + self.rest.len() as u64;
        let skipping = std::mem::replace(&mut self.skipping, false);
        let text = self.encoding.decode(&std::mem::take(&mut self.rest));
        self.start = end;
        self.flushed = true;
        if skipping { None } else { Some(RawLine { text, end }) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(s: &str) -> Vec<u8> { s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect() }

    #[test]
    fn decodage() {
        assert_eq!(Encoding::Utf8.decode("é\r".as_bytes()), "é");
        assert_eq!(Encoding::Utf8.decode(&[0x61, 0xFF]), "a\u{FFFD}");
        assert_eq!(Encoding::Latin1.decode(&[0xE9, 0x80]), "é\u{80}");
        assert_eq!(Encoding::Windows1252.decode(&[0x80, 0x9C, 0x93, 0x94, 0xE9]), "€œ“”é");
        assert_eq!(Encoding::Utf16Le.decode(&utf16le("été")), "été");
        assert_eq!(Encoding::parse("UTF-16"), Some(Encoding::Utf16Le));
        assert_eq!(Encoding::parse("cp1252"), Some(Encoding::Windows1252));
        assert_eq!(Encoding::parse("ebcdic"), None);
        assert_eq!(detect_bom(&[0xFF, 0xFE, 0x61, 0]), Some((Encoding::Utf16Le, 2)));
        assert_eq!(detect_bom(b"abc"), None);
    }

    #[test]
    fn decoupage_et_positions() {
        let mut s = ByteSplitter::new(Encoding::Utf8, 10);
        assert!(s.push(b"ab").is_empty());
        let lines = s.push(b"c\r\nde\nf");
        assert_eq!(lines, vec![RawLine { text: "abc".into(), end: 15 }, RawLine { text: "de".into(), end: 18 }]);
        assert_eq!((s.position(), s.pending()), (18, 1));
        assert_eq!(s.flush(), Some(RawLine { text: "f".into(), end: 19 }));
        // La fin de ligne de « f », arrivée après, ne crée pas de ligne vide.
        assert_eq!(s.push(b"\ng\n"), vec![RawLine { text: "g".into(), end: 22 }]);
        assert_eq!(s.push(b"\n"), vec![RawLine { text: "".into(), end: 23 }]);

        // UTF-16 : la fin de ligne est sur 2 octets, alignée.
        let mut s = ByteSplitter::new(Encoding::Utf16Le, 2);
        let bytes = utf16le("ligne ☃\r\nsuite\n");
        let (a, b) = bytes.split_at(5);
        assert!(s.push(a).is_empty());
        let lines = s.push(b);
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["ligne ☃", "suite"]);
        assert_eq!(lines[1].end, 2 + bytes.len() as u64);
    }

    #[test]
    fn ligne_trop_longue() {
        let mut s = ByteSplitter::new(Encoding::Utf8, 0);
        let long = vec![b'x'; MAX_LINE_BYTES + 10];
        let lines = s.push(&long);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.ends_with(TRUNCATED) && lines[0].text.len() == MAX_LINE_BYTES + TRUNCATED.len());
        // La suite de la ligne est ignorée jusqu'à sa fin.
        let lines = s.push(b"yyy\nsuivante\n");
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["suivante"]);
        assert_eq!(s.position(), MAX_LINE_BYTES as u64 + 10 + 13);
    }
}
