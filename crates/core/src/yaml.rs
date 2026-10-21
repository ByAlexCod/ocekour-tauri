//! YAML de `config.yaml` : lecture (schéma « core » YAML 1.2, positions de chaque nœud) et
//! modification **chirurgicale** du texte : seule la partie modifiée est réécrite, les
//! commentaires et la mise en forme de l'utilisateur sont conservés.

use saphyr_parser::{Event, Parser, ScalarStyle, Span};
use serde_json::{Map, Number, Value};

/// Erreur de syntaxe positionnée (lignes et colonnes à partir de 1).
#[derive(Debug, Clone, PartialEq)]
pub struct YamlError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Scalar { text: String, style: ScalarStyle, tagged_str: bool },
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
    Alias(usize),
}

/// Nœud du document : positions en octets dans le texte, ligne / colonne à partir de 1.
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
    pub flow: bool,
    pub anchor: usize,
}

/// Segment de chemin dans le document.
#[derive(Debug, Clone, PartialEq)]
pub enum P {
    K(String),
    I(usize),
}

impl From<&str> for P {
    fn from(s: &str) -> Self { P::K(s.to_string()) }
}
impl From<String> for P {
    fn from(s: String) -> Self { P::K(s) }
}
impl From<usize> for P {
    fn from(i: usize) -> Self { P::I(i) }
}

/// Construit un chemin : `path!["profiles", 0, "fields"]`.
#[macro_export]
macro_rules! ypath {
    ($($x:expr),* $(,)?) => { vec![$($crate::yaml::P::from($x)),*] };
}

/// Document analysé.
#[derive(Debug)]
pub struct Doc {
    pub text: String,
    pub root: Option<Node>,
}

/// Convertit les positions en caractères du lecteur en octets.
struct Offsets {
    /// Octet de chaque caractère (et un de plus en fin).
    bytes: Vec<usize>,
}

impl Offsets {
    fn new(text: &str) -> Self {
        let mut bytes: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        bytes.push(text.len());
        Offsets { bytes }
    }
    fn at(&self, chars: usize) -> usize {
        self.bytes.get(chars).copied().unwrap_or(*self.bytes.last().unwrap())
    }
}

enum Frame {
    Seq(Node, Vec<Node>),
    Map(Node, Vec<(Node, Node)>, Option<Node>),
}

/// Analyse le texte (premier document seulement).
pub fn parse(text: &str) -> Result<Doc, YamlError> {
    let offsets = Offsets::new(text);
    let mut parser = Parser::new_from_str(text);
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Node> = None;
    let mut docs = 0;
    let node_at = |span: &Span, kind: NodeKind, flow: bool, anchor: usize| Node {
        start: offsets.at(span.start.index()),
        end: offsets.at(span.end.index()),
        line: span.start.line(),
        col: span.start.col() + 1,
        kind,
        flow,
        anchor,
    };
    let mut finished: Option<Node>;
    for item in parser.by_ref() {
        let (event, span) = item.map_err(|e| {
            let m = e.marker();
            YamlError { message: e.info().to_string(), line: m.line(), col: m.col() + 1 }
        })?;
        finished = None;
        match event {
            Event::DocumentStart(_) => {
                docs += 1;
                if docs > 1 { break; }
            }
            Event::Scalar(value, style, anchor, tag) => {
                let tagged_str = tag.as_ref().map(|t| t.suffix == "str").unwrap_or(false);
                let mut n = node_at(&span, NodeKind::Scalar { text: value.to_string(), style, tagged_str }, false, anchor);
                if matches!(style, ScalarStyle::Literal | ScalarStyle::Folded) {
                    // Bloc : se termine à la fin de son contenu.
                    n.end = trim_end_ws(text, n.end);
                }
                finished = Some(n);
            }
            Event::Alias(id) => finished = Some(node_at(&span, NodeKind::Alias(id), false, 0)),
            Event::SequenceStart(anchor, _) => {
                let flow = text[offsets.at(span.start.index())..].starts_with('[');
                stack.push(Frame::Seq(node_at(&span, NodeKind::Seq(vec![]), flow, anchor), vec![]));
            }
            Event::MappingStart(anchor, _) => {
                let flow = text[offsets.at(span.start.index())..].starts_with('{');
                stack.push(Frame::Map(node_at(&span, NodeKind::Map(vec![]), flow, anchor), vec![], None));
            }
            Event::SequenceEnd => {
                if let Some(Frame::Seq(mut n, items)) = stack.pop() {
                    n.end = if n.flow {
                        offsets.at(span.end.index())
                    } else {
                        items.last().map(|i| i.end).unwrap_or(n.start)
                    };
                    if n.flow && !text[..n.end].ends_with(']') {
                        n.end = text[n.end..].find(']').map(|i| n.end + i + 1).unwrap_or(n.end);
                    }
                    n.kind = NodeKind::Seq(items);
                    finished = Some(n);
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map(mut n, entries, _)) = stack.pop() {
                    n.end = if n.flow {
                        offsets.at(span.end.index())
                    } else {
                        entries.last().map(|(_, v)| v.end).unwrap_or(n.start)
                    };
                    if n.flow && !text[..n.end].ends_with('}') {
                        n.end = text[n.end..].find('}').map(|i| n.end + i + 1).unwrap_or(n.end);
                    }
                    // Clés en double : refusées (comme le module yaml d'Ocekour 1.x).
                    for (i, (k, _)) in entries.iter().enumerate() {
                        if let NodeKind::Scalar { text: key, .. } = &k.kind {
                            if entries[..i].iter().any(|(o, _)| matches!(&o.kind, NodeKind::Scalar { text, .. } if text == key)) {
                                return Err(YamlError { message: "Map keys must be unique".into(), line: k.line, col: k.col });
                            }
                        }
                    }
                    n.kind = NodeKind::Map(entries);
                    finished = Some(n);
                }
            }
            _ => {}
        }
        if let Some(node) = finished {
            match stack.last_mut() {
                Some(Frame::Seq(_, items)) => items.push(node),
                Some(Frame::Map(_, entries, key)) => match key.take() {
                    None => *key = Some(node),
                    Some(k) => entries.push((k, node)),
                },
                None => {
                    if root.is_none() { root = Some(node); }
                }
            }
        }
    }
    // Document vide ou seulement des commentaires : aucun contenu.
    if let Some(n) = &root {
        if let NodeKind::Scalar { text: t, style: ScalarStyle::Plain, .. } = &n.kind {
            if t.is_empty() { root = None; }
        }
    }
    Ok(Doc { text: text.to_string(), root })
}

fn trim_end_ws(text: &str, mut end: usize) -> usize {
    while end > 0 && text.as_bytes()[end - 1].is_ascii_whitespace() { end -= 1; }
    end
}

/// Valeur JSON d'un scalaire « plain » selon le schéma core de YAML 1.2.
fn plain_value(text: &str) -> Value {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Value::from(f64::INFINITY),
        _ => {}
    }
    let int = |digits: &str, radix: u32| i64::from_str_radix(digits, radix).ok().map(|n| Value::Number(n.into()));
    if let Some(hex) = text.strip_prefix("0x") {
        if let Some(v) = int(hex, 16) { return v; }
    }
    if let Some(oct) = text.strip_prefix("0o") {
        if let Some(v) = int(oct, 8) { return v; }
    }
    let bytes = text.as_bytes();
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = text.parse::<i64>() { return Value::Number(n.into()); }
        if let Ok(f) = text.parse::<f64>() { return Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null); }
    }
    let float_like = regex_float(text);
    if float_like {
        if let Ok(f) = text.parse::<f64>() {
            return Number::from_f64(f).map(Value::Number).unwrap_or_else(|| Value::String(text.into()));
        }
    }
    let _ = bytes;
    Value::String(text.to_string())
}

fn regex_float(text: &str) -> bool {
    static RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$").unwrap());
    RE.is_match(text)
}

impl Node {
    /// Valeur JSON du nœud (alias résolus).
    pub fn to_json(&self, anchors: &mut std::collections::HashMap<usize, Value>) -> Value {
        let value = match &self.kind {
            NodeKind::Scalar { text, style, tagged_str } => {
                if *tagged_str || !matches!(style, ScalarStyle::Plain) {
                    Value::String(text.clone())
                } else {
                    plain_value(text)
                }
            }
            NodeKind::Seq(items) => Value::Array(items.iter().map(|i| i.to_json(anchors)).collect()),
            NodeKind::Map(entries) => {
                let mut map = Map::new();
                for (k, v) in entries {
                    let key = match k.to_json(anchors) {
                        Value::String(s) => s,
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    let value = v.to_json(anchors);
                    if key == "<<" {
                        // Fusion (`<<: *defaut`) : les clés présentes priment.
                        if let Value::Object(base) = value {
                            for (bk, bv) in base { map.entry(bk).or_insert(bv); }
                        }
                        continue;
                    }
                    map.insert(key, value);
                }
                Value::Object(map)
            }
            NodeKind::Alias(id) => anchors.get(id).cloned().unwrap_or(Value::Null),
        };
        if self.anchor != 0 { anchors.insert(self.anchor, value.clone()); }
        value
    }

    fn key_text(&self) -> Option<&str> {
        match &self.kind { NodeKind::Scalar { text, .. } => Some(text), _ => None }
    }

    /// Enfant direct (clé de map ou index de liste).
    pub fn child(&self, p: &P) -> Option<&Node> {
        match (&self.kind, p) {
            (NodeKind::Map(entries), P::K(k)) => entries.iter().find(|(key, _)| key.key_text() == Some(k)).map(|(_, v)| v),
            (NodeKind::Seq(items), P::I(i)) => items.get(*i),
            _ => None,
        }
    }

    fn entry(&self, key: &str) -> Option<&(Node, Node)> {
        match &self.kind {
            NodeKind::Map(entries) => entries.iter().find(|(k, _)| k.key_text() == Some(key)),
            _ => None,
        }
    }

    fn is_collection(&self) -> bool {
        matches!(self.kind, NodeKind::Seq(_) | NodeKind::Map(_))
    }
}

impl Doc {
    pub fn to_json(&self) -> Value {
        match &self.root {
            Some(n) => n.to_json(&mut Default::default()),
            None => Value::Null,
        }
    }

    /// Nœud au chemin donné.
    pub fn node(&self, path: &[P]) -> Option<&Node> {
        let mut node = self.root.as_ref()?;
        for p in path { node = node.child(p)?; }
        Some(node)
    }

    /// Position du nœud le plus profond existant sur ce chemin (erreurs de schéma).
    pub fn position(&self, path: &[P]) -> Option<(usize, usize)> {
        let mut len = path.len();
        loop {
            if let Some(n) = self.node(&path[..len]) { return Some((n.line, n.col)); }
            if len == 0 { return None; }
            len -= 1;
        }
    }
}

// --- Écriture ---------------------------------------------------------------------------------

/// Texte YAML d'un scalaire (entre guillemets si nécessaire).
pub fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => string_scalar(s),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn string_scalar(s: &str) -> String {
    let plain_ok = !s.is_empty()
        && s.trim() == s
        && !s.contains(['\n', '\r', '\t'])
        && !s.chars().any(|c| c.is_control())
        && !s.starts_with(['-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`'])
        && !s.contains(": ")
        && !s.contains(" #")
        && !s.ends_with(':')
        && matches!(plain_value(s), Value::String(_));
    if plain_ok {
        return s.to_string();
    }
    if !s.contains(['\n', '\r', '\t', '\'', '\\']) && !s.chars().any(|c| c.is_control()) {
        return format!("'{s}'");
    }
    serde_json::to_string(s).unwrap()
}

fn inline(v: &Value) -> Option<String> {
    match v {
        Value::Array(a) if a.is_empty() => Some("[]".into()),
        Value::Object(o) if o.is_empty() => Some("{}".into()),
        Value::Array(_) | Value::Object(_) => None,
        other => Some(scalar(other)),
    }
}

/// Texte après `clé:` pour une valeur (inline ou bloc indenté à `indent`).
fn after_key(v: &Value, indent: usize) -> String {
    match inline(v) {
        Some(s) => format!(" {s}\n"),
        None => match v {
            Value::Object(o) => format!("\n{}", map_lines(o, indent + 2)),
            Value::Array(a) => format!("\n{}", seq_lines(a, indent + 2)),
            _ => unreachable!(),
        },
    }
}

fn map_lines(o: &Map<String, Value>, indent: usize) -> String {
    let pad = " ".repeat(indent);
    o.iter().map(|(k, v)| format!("{pad}{}:{}", string_scalar(k), after_key(v, indent))).collect()
}

fn seq_lines(a: &[Value], indent: usize) -> String {
    let pad = " ".repeat(indent);
    a.iter().map(|v| format!("{pad}- {}", seq_item(v, indent))).collect()
}

/// Élément de liste (après `- `), indenté à `indent` (colonne du tiret).
fn seq_item(v: &Value, indent: usize) -> String {
    match inline(v) {
        Some(s) => format!("{s}\n"),
        None => match v {
            Value::Object(o) => {
                let lines = map_lines(o, indent + 2);
                lines[indent + 2..].to_string()
            }
            Value::Array(a) => {
                let lines = seq_lines(a, indent + 2);
                lines[indent + 2..].to_string()
            }
            _ => unreachable!(),
        },
    }
}

/// Rendu complet d'un document (racine).
pub fn render(v: &Value) -> String {
    match v {
        Value::Object(o) if !o.is_empty() => map_lines(o, 0),
        Value::Array(a) if !a.is_empty() => seq_lines(a, 0),
        other => format!("{}\n", inline(other).unwrap_or_default()),
    }
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

/// Fin de la ligne qui contient `pos` (après le `\n`).
fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map(|i| pos + i + 1).unwrap_or(text.len())
}

/// Colonne (0) d'une position.
fn column(text: &str, pos: usize) -> usize {
    text[line_start(text, pos)..pos].chars().count()
}

/// Position du `-` d'un élément de liste en bloc.
fn dash_before(text: &str, item_start: usize) -> usize {
    let before = &text[..item_start];
    let trimmed = before.trim_end_matches([' ', '\t']);
    if trimmed.ends_with('-') { trimmed.len() - 1 } else { item_start }
}

/// Fin de la clé (après le `:`).
fn colon_after(text: &str, key: &Node) -> usize {
    let mut i = key.end;
    let bytes = text.as_bytes();
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'"' || bytes[i] == b'\'') { i += 1; }
    if i < bytes.len() && bytes[i] == b':' { i + 1 } else { key.end }
}

/// Erreur d'édition (document invalide, chemin impossible).
#[derive(Debug)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for EditError {}

fn edit_err(msg: &str) -> EditError { EditError(msg.into()) }

/// Document modifiable : chaque opération relit le texte et n'en réécrit qu'une partie.
pub struct Editor {
    pub text: String,
}

impl Editor {
    pub fn new(text: &str) -> Self { Editor { text: text.to_string() } }

    fn doc(&self) -> Result<Doc, EditError> {
        parse(&self.text).map_err(|e| EditError(e.message))
    }

    pub fn get(&self, path: &[P]) -> Option<Value> {
        let doc = self.doc().ok()?;
        Some(doc.node(path)?.to_json(&mut Default::default()))
    }

    fn splice(&mut self, start: usize, end: usize, with: &str) {
        self.text.replace_range(start..end, with);
    }

    /// Valeur actuelle (JSON) à un chemin, pour les replis « tout réécrire ».
    fn json_at(&self, doc: &Doc, path: &[P]) -> Value {
        doc.node(path).map(|n| n.to_json(&mut Default::default())).unwrap_or(Value::Null)
    }

    /// Remplace le document entier (racine absente ou non éditable finement).
    fn rewrite_root(&mut self, doc: &Doc, value: &Value) {
        match &doc.root {
            None => {
                let mut text = self.text.clone();
                if !text.is_empty() && !text.ends_with('\n') { text.push('\n'); }
                text.push_str(&render(value));
                self.text = text;
            }
            Some(root) => {
                let start = line_start(&self.text, root.start);
                let end = line_end(&self.text, root.end);
                let rendered = render(value);
                self.splice(start, end, &rendered);
            }
        }
    }

    /// Fixe une valeur (crée les maps intermédiaires).
    pub fn set(&mut self, path: &[P], value: &Value) -> Result<(), EditError> {
        let doc = self.doc()?;
        if path.is_empty() {
            self.rewrite_root(&doc, value);
            return Ok(());
        }
        let Some(root) = &doc.root else {
            let v = nest(path, value)?;
            self.rewrite_root(&doc, &v);
            return Ok(());
        };
        // Nœud existant le plus profond.
        let mut node = root;
        let mut depth = 0;
        while depth < path.len() {
            match node.child(&path[depth]) {
                Some(child) => { node = child; depth += 1; }
                None => break,
            }
        }
        if depth == path.len() {
            return self.replace(&doc, path, value);
        }
        // Parent existant : `node` (chemin path[..depth]), clé manquante path[depth].
        let rest = nest(&path[depth + 1..], value)?;
        match (&node.kind, &path[depth]) {
            (NodeKind::Map(entries), P::K(key)) if !node.flow && !entries.is_empty() => {
                let indent = column(&self.text, entries[0].0.start);
                let at = line_end(&self.text, node.end);
                let mut insert = String::new();
                if at == self.text.len() && !self.text.ends_with('\n') { insert.push('\n'); }
                insert.push_str(&format!("{}{}:{}", " ".repeat(indent), string_scalar(key), after_key(&rest, indent)));
                self.splice(at, at, &insert);
                Ok(())
            }
            (NodeKind::Seq(items), P::I(i)) if *i == items.len() => self.push(&path[..depth], &rest),
            (NodeKind::Map(_), P::K(key)) => {
                // Map vide ou en style flow : réécrite en entier.
                let mut current = self.json_at(&doc, &path[..depth]);
                current.as_object_mut().unwrap().insert(key.clone(), rest);
                self.replace(&doc, &path[..depth], &current)
            }
            (NodeKind::Scalar { .. }, _) if depth > 0 => {
                // `clé:` sans valeur (null) : devient une map.
                let v = nest(&path[depth..], value)?;
                self.replace(&doc, &path[..depth], &v)
            }
            (NodeKind::Scalar { .. }, _) => {
                let v = nest(path, value)?;
                self.rewrite_root(&doc, &v);
                Ok(())
            }
            _ => Err(edit_err("Chemin YAML impossible")),
        }
    }

    /// Remplace la valeur d'un nœud existant.
    fn replace(&mut self, doc: &Doc, path: &[P], value: &Value) -> Result<(), EditError> {
        if path.is_empty() {
            self.rewrite_root(doc, value);
            return Ok(());
        }
        let parent = doc.node(&path[..path.len() - 1]).ok_or_else(|| edit_err("Chemin YAML introuvable"))?;
        let old = doc.node(path).unwrap();
        if parent.flow {
            let mut current = self.json_at(doc, &path[..path.len() - 1]);
            match (&mut current, path.last().unwrap()) {
                (Value::Object(o), P::K(k)) => { o.insert(k.clone(), value.clone()); }
                (Value::Array(a), P::I(i)) => { a[*i] = value.clone(); }
                _ => return Err(edit_err("Chemin YAML impossible")),
            }
            return self.replace(doc, &path[..path.len() - 1], &current);
        }
        match (&parent.kind, path.last().unwrap()) {
            (NodeKind::Map(_), P::K(key)) => {
                let (k, _) = parent.entry(key).unwrap();
                let indent = column(&self.text, k.start);
                let from = colon_after(&self.text, k);
                match inline(value) {
                    Some(s) if !old.is_collection() || old.flow => {
                        // Valeur sur la même ligne : le commentaire de fin de ligne est gardé.
                        let block = matches!(&old.kind, NodeKind::Scalar { style: ScalarStyle::Literal | ScalarStyle::Folded, .. });
                        let end = if block { trim_end_ws(&self.text, old.end) } else { old.end };
                        self.splice(from, end, &format!(" {s}"));
                    }
                    _ => {
                        let end = line_end(&self.text, old.end);
                        let rendered = after_key(value, indent);
                        self.splice(from, end, &rendered);
                    }
                }
                Ok(())
            }
            (NodeKind::Seq(_), P::I(_)) => {
                let dash = dash_before(&self.text, old.start);
                let indent = column(&self.text, dash);
                match inline(value) {
                    Some(s) if !old.is_collection() || old.flow => self.splice(old.start, old.end, &s),
                    _ => {
                        let end = line_end(&self.text, old.end);
                        let rendered = seq_item(value, indent);
                        self.splice(old.start, end, &rendered);
                    }
                }
                Ok(())
            }
            _ => Err(edit_err("Chemin YAML impossible")),
        }
    }

    /// Supprime une clé ou un élément (sans erreur s'il n'existe pas).
    pub fn delete(&mut self, path: &[P]) -> Result<(), EditError> {
        let doc = self.doc()?;
        let Some(old) = doc.node(path) else { return Ok(()) };
        if path.is_empty() {
            self.text.clear();
            return Ok(());
        }
        let parent_path = &path[..path.len() - 1];
        let parent = doc.node(parent_path).unwrap();
        let count = match &parent.kind {
            NodeKind::Map(e) => e.len(),
            NodeKind::Seq(i) => i.len(),
            _ => 0,
        };
        if parent.flow || (count == 1 && !parent_path.is_empty()) {
            let mut current = self.json_at(&doc, parent_path);
            match (&mut current, path.last().unwrap()) {
                (Value::Object(o), P::K(k)) => { o.shift_remove(k); }
                (Value::Array(a), P::I(i)) => { a.remove(*i); }
                _ => {}
            }
            return self.replace(&doc, parent_path, &current);
        }
        let start = match (&parent.kind, path.last().unwrap()) {
            (NodeKind::Map(_), P::K(key)) => parent.entry(key).unwrap().0.start,
            _ => dash_before(&self.text, old.start),
        };
        let from = line_start(&self.text, start);
        let to = line_end(&self.text, old.end.max(start));
        self.splice(from, to, "");
        Ok(())
    }

    /// Ajoute en fin de liste (la crée au besoin).
    pub fn push(&mut self, path: &[P], value: &Value) -> Result<(), EditError> {
        self.add(path, value, false)
    }

    /// Ajoute en tête de liste (la crée au besoin).
    pub fn unshift(&mut self, path: &[P], value: &Value) -> Result<(), EditError> {
        self.add(path, value, true)
    }

    fn add(&mut self, path: &[P], value: &Value, first: bool) -> Result<(), EditError> {
        let doc = self.doc()?;
        let node = doc.node(path);
        let block_items = match node.map(|n| (&n.kind, n.flow)) {
            Some((NodeKind::Seq(items), false)) if !items.is_empty() => Some(items),
            _ => None,
        };
        let Some(items) = block_items else {
            let mut list = match node.map(|n| n.to_json(&mut Default::default())) {
                Some(Value::Array(a)) => a,
                _ => vec![],
            };
            if first { list.insert(0, value.clone()) } else { list.push(value.clone()) }
            return self.set(path, &Value::Array(list));
        };
        let dash = dash_before(&self.text, items[0].start);
        let indent = column(&self.text, dash);
        let item = format!("{}- {}", " ".repeat(indent), seq_item(value, indent));
        if first {
            let at = line_start(&self.text, dash);
            self.splice(at, at, &item);
        } else {
            let at = line_end(&self.text, node.unwrap().end);
            let prefix = if at == self.text.len() && !self.text.ends_with('\n') { "\n" } else { "" };
            self.splice(at, at, &format!("{prefix}{item}"));
        }
        Ok(())
    }
}

/// `{ a: { b: value } }` pour le chemin `[a, b]`.
fn nest(path: &[P], value: &Value) -> Result<Value, EditError> {
    let mut v = value.clone();
    for p in path.iter().rev() {
        match p {
            P::K(k) => {
                let mut m = Map::new();
                m.insert(k.clone(), v);
                v = Value::Object(m);
            }
            P::I(0) => v = Value::Array(vec![v]),
            P::I(_) => return Err(edit_err("Index de liste hors limites")),
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edit(text: &str, f: impl FnOnce(&mut Editor)) -> String {
        let mut e = Editor::new(text);
        f(&mut e);
        parse(&e.text).expect("YAML valide après édition");
        e.text
    }

    #[test]
    fn lecture_schema_core() {
        let doc = parse("a: 1\nb: yes\nc: true\nd: ~\ne: '1'\nf: 1.5\ng: 0x10\nh: C:\\certs\\ca.cer\ni: [x, 'y']\n").unwrap();
        assert_eq!(doc.to_json(), json!({
            "a": 1, "b": "yes", "c": true, "d": null, "e": "1", "f": 1.5, "g": 16,
            "h": "C:\\certs\\ca.cer", "i": ["x", "y"]
        }));
    }

    #[test]
    fn positions_en_octets_avec_accents() {
        let text = "# réglages éàü\nthème: clair\nconnections:\n  - id: a\n    url: pas-une-url\n";
        let doc = parse(text).unwrap();
        let n = doc.node(&ypath!["connections", 0, "url"]).unwrap();
        assert_eq!(&text[n.start..n.end], "pas-une-url");
        assert_eq!((n.line, n.col), (5, 10));
        let n = doc.node(&ypath!["thème"]).unwrap();
        assert_eq!(&text[n.start..n.end], "clair");
    }

    #[test]
    fn erreurs_de_syntaxe_et_doublons() {
        let e = parse("theme: dark\nprofiles: [\n").unwrap_err();
        assert!(e.line > 0);
        let e = parse("a: 1\na: 2\n").unwrap_err();
        assert_eq!((e.message.as_str(), e.line), ("Map keys must be unique", 2));
    }

    #[test]
    fn document_vide_ou_commentaires() {
        assert!(parse("").unwrap().root.is_none());
        assert!(parse("# rien\n").unwrap().root.is_none());
    }

    #[test]
    fn remplace_un_scalaire_en_gardant_le_commentaire() {
        let out = edit("# mes réglages\ntheme: light # clair le jour\nx: 1\n", |e| {
            e.set(&ypath!["theme"], &json!("dark")).unwrap()
        });
        assert_eq!(out, "# mes réglages\ntheme: dark # clair le jour\nx: 1\n");
    }

    #[test]
    fn ajoute_une_cle_racine_et_imbriquee() {
        let out = edit("# c\ntheme: dark\n", |e| {
            e.set(&ypath!["bufferSize"], &json!(5000)).unwrap();
            e.set(&ypath!["services", "gitlab", "url"], &json!("https://gitlab.intra")).unwrap();
        });
        assert_eq!(out, "# c\ntheme: dark\nbufferSize: 5000\nservices:\n  gitlab:\n    url: https://gitlab.intra\n");
        let out = edit("", |e| e.set(&ypath!["favorites"], &json!(["a/b"])).unwrap());
        assert_eq!(out, "favorites:\n  - a/b\n");
        let out = edit("theme: dark", |e| e.set(&ypath!["x"], &json!(true)).unwrap());
        assert_eq!(out, "theme: dark\nx: true\n");
    }

    #[test]
    fn liste_push_unshift_et_suppression() {
        let text = "profiles:\n  # premier\n  - match: '*'\n    columns: [a]\nautre: 1\n";
        let out = edit(text, |e| {
            e.unshift(&ypath!["profiles"], &json!({ "match": "prod/ns/api" })).unwrap();
            e.push(&ypath!["profiles"], &json!({ "match": "x", "expand": ["s"] })).unwrap();
        });
        assert_eq!(
            out,
            "profiles:\n  # premier\n  - match: prod/ns/api\n  - match: '*'\n    columns: [a]\n  - match: x\n    expand:\n      - s\nautre: 1\n"
        );
        let doc = parse(&out).unwrap();
        assert_eq!(doc.to_json()["profiles"][0], json!({ "match": "prod/ns/api" }));
        let out = edit(&out, |e| e.delete(&ypath!["profiles", 1]).unwrap());
        assert_eq!(parse(&out).unwrap().to_json()["profiles"], json!([{ "match": "prod/ns/api" }, { "match": "x", "expand": ["s"] }]));
    }

    #[test]
    fn liste_en_flow_et_creation() {
        let out = edit("profiles:\n  - match: a\n    columns: [x]\n", |e| {
            e.set(&ypath!["profiles", 0, "columns"], &json!(["x", "y"])).unwrap();
        });
        assert_eq!(out, "profiles:\n  - match: a\n    columns:\n      - x\n      - y\n");
        let out = edit("theme: dark\n", |e| e.push(&ypath!["connections"], &json!({ "id": "k", "brokers": ["a:1"] })).unwrap());
        assert_eq!(out, "theme: dark\nconnections:\n  - id: k\n    brokers:\n      - a:1\n");
    }

    #[test]
    fn supprime_une_cle_et_un_dernier_element() {
        let out = edit("a: 1\nfields:\n  time: t # c\n  level: l\nb: 2\n", |e| {
            e.delete(&ypath!["fields", "time"]).unwrap();
        });
        assert_eq!(out, "a: 1\nfields:\n  level: l\nb: 2\n");
        let out = edit(&out, |e| e.delete(&ypath!["fields", "level"]).unwrap());
        assert_eq!(parse(&out).unwrap().to_json(), json!({ "a": 1, "fields": {}, "b": 2 }));
        let out = edit("a: 1\nb: 2\n", |e| e.delete(&ypath!["a"]).unwrap());
        assert_eq!(out, "b: 2\n");
        let out = edit("a: 1\n", |e| e.delete(&ypath!["zz"]).unwrap());
        assert_eq!(out, "a: 1\n");
    }

    #[test]
    fn remplace_une_map_par_une_autre() {
        let out = edit("notifications:\n  enabled: false # off\n  target: self\nz: 1\n", |e| {
            e.set(&ypath!["notifications"], &json!({ "enabled": true, "target": "#canal" })).unwrap();
        });
        assert_eq!(out, "notifications:\n  enabled: true\n  target: '#canal'\nz: 1\n");
    }

    #[test]
    fn element_de_liste_map_modifie_cle_par_cle() {
        let text = "connections:\n  - id: r1 # ma connexion\n    type: rancher\n    name: A\n";
        let out = edit(text, |e| {
            e.set(&ypath!["connections", 0, "name"], &json!("Renommée")).unwrap();
            e.set(&ypath!["connections", 0, "tls"], &json!({ "insecure": true })).unwrap();
        });
        assert_eq!(out, "connections:\n  - id: r1 # ma connexion\n    type: rancher\n    name: Renommée\n    tls:\n      insecure: true\n");
    }

    #[test]
    fn scalaires_cites_si_ambigus() {
        assert_eq!(scalar(&json!("true")), "'true'");
        assert_eq!(scalar(&json!("123")), "'123'");
        assert_eq!(scalar(&json!("")), "''");
        assert_eq!(scalar(&json!("a: b")), "'a: b'");
        assert_eq!(scalar(&json!("ligne\nsuite")), "\"ligne\\nsuite\"");
        assert_eq!(scalar(&json!("*")), "'*'");
        assert_eq!(scalar(&json!("prod/*/api")), "prod/*/api");
        assert_eq!(scalar(&json!("it's")), "it's");
        for s in ["true", "123", "", "a: b", "ligne\nsuite", "*", "#x", "it's", "- a", "null", "1.5", "x #y"] {
            let doc = parse(&format!("k: {}\n", scalar(&json!(s)))).unwrap();
            assert_eq!(doc.to_json()["k"], json!(s), "{s}");
        }
    }

    #[test]
    fn bloc_litteral() {
        let doc = parse("a: |\n  ligne 1\n  ligne 2\nb: 1\n").unwrap();
        assert_eq!(doc.to_json()["a"], json!("ligne 1\nligne 2\n"));
        let out = edit("a: |\n  ligne 1\nb: 1\n", |e| e.set(&ypath!["a"], &json!("x")).unwrap());
        assert_eq!(parse(&out).unwrap().to_json(), json!({ "a": "x", "b": 1 }));
    }
}
