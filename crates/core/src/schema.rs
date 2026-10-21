//! Validation de données JSON (à la manière de zod) : valeurs par défaut, clés inconnues
//! retirées, erreurs en français avec leur chemin, et export en JSON Schema (outils MCP).
//!
//! Une même description sert à valider `config.yaml`, les brouillons de l'interface et les
//! arguments des outils MCP.

use serde_json::{Map, Value, json};
use std::sync::Arc;

/// Segment de chemin d'une erreur (`connections.0.url`).
#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

impl std::fmt::Display for Seg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Seg::Key(k) => write!(f, "{k}"),
            Seg::Index(i) => write!(f, "{i}"),
        }
    }
}

/// Erreur de validation : chemin et message lisible.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub path: Vec<Seg>,
    pub message: String,
}

impl Issue {
    pub fn path_string(&self) -> String {
        self.path.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(".")
    }
    /// « chemin : message » (ou le message seul à la racine).
    pub fn describe(&self) -> String {
        let where_ = self.path_string();
        if where_.is_empty() { self.message.clone() } else { format!("{where_} : {}", self.message) }
    }
}

type Refine = Arc<dyn Fn(&Value) -> Option<String> + Send + Sync>;
type Transform = Arc<dyn Fn(Value) -> Value + Send + Sync>;

#[derive(Clone)]
pub enum Kind {
    Any,
    Str { min: Option<usize>, max: Option<usize>, url: bool, pattern: Option<regex::Regex>, trim: bool },
    Int { min: Option<i64>, max: Option<i64> },
    Num { min: Option<f64>, max: Option<f64> },
    Bool,
    Enum(Vec<String>),
    Literal(Value),
    Array { item: Box<S>, min: Option<usize>, max: Option<usize> },
    Tuple { items: Vec<S>, rest: Option<Box<S>> },
    Object { fields: Vec<(String, S)>, loose: bool },
    Record(Box<S>),
    /// Union discriminée par une clé (`type`, `op`…).
    Disc { key: String, variants: Vec<(String, S)> },
    /// Première variante qui valide.
    Union(Vec<S>),
}

#[derive(Clone)]
enum Default_ {
    None,
    Value(Value),
}

/// Description d'une valeur attendue.
#[derive(Clone)]
pub struct S {
    pub kind: Kind,
    optional: bool,
    nullable: bool,
    default: Default_,
    description: Option<String>,
    refine: Option<Refine>,
    transform: Option<Transform>,
}

fn s(kind: Kind) -> S {
    S { kind, optional: false, nullable: false, default: Default_::None, description: None, refine: None, transform: None }
}

pub fn any() -> S { s(Kind::Any) }
pub fn string() -> S { s(Kind::Str { min: None, max: None, url: false, pattern: None, trim: false }) }
pub fn int() -> S { s(Kind::Int { min: None, max: None }) }
pub fn number() -> S { s(Kind::Num { min: None, max: None }) }
pub fn boolean() -> S { s(Kind::Bool) }
pub fn enumeration(values: &[&str]) -> S { s(Kind::Enum(values.iter().map(|v| v.to_string()).collect())) }
pub fn literal(value: Value) -> S { s(Kind::Literal(value)) }
pub fn array(item: S) -> S { s(Kind::Array { item: Box::new(item), min: None, max: None }) }
pub fn tuple(items: Vec<S>, rest: Option<S>) -> S { s(Kind::Tuple { items, rest: rest.map(Box::new) }) }
pub fn record(value: S) -> S { s(Kind::Record(Box::new(value))) }
pub fn union(variants: Vec<S>) -> S { s(Kind::Union(variants)) }
pub fn object(fields: Vec<(&str, S)>) -> S {
    s(Kind::Object { fields: fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), loose: false })
}
/// Union discriminée : chaque variante est un objet dont `key` est un littéral.
pub fn disc(key: &str, variants: Vec<(&str, S)>) -> S {
    s(Kind::Disc { key: key.into(), variants: variants.into_iter().map(|(k, v)| (k.to_string(), v)).collect() })
}

impl S {
    pub fn optional(mut self) -> Self { self.optional = true; self }
    pub fn nullable(mut self) -> Self { self.nullable = true; self }
    pub fn default(mut self, value: Value) -> Self { self.default = Default_::Value(value); self }
    pub fn describe(mut self, text: &str) -> Self { self.description = Some(text.into()); self }
    pub fn refine(mut self, f: impl Fn(&Value) -> Option<String> + Send + Sync + 'static) -> Self {
        self.refine = Some(Arc::new(f));
        self
    }
    pub fn transform(mut self, f: impl Fn(Value) -> Value + Send + Sync + 'static) -> Self {
        self.transform = Some(Arc::new(f));
        self
    }
    pub fn min(mut self, n: i64) -> Self {
        match &mut self.kind {
            Kind::Str { min, .. } => *min = Some(n as usize),
            Kind::Int { min, .. } => *min = Some(n),
            Kind::Num { min, .. } => *min = Some(n as f64),
            Kind::Array { min, .. } => *min = Some(n as usize),
            _ => {}
        }
        self
    }
    pub fn max(mut self, n: i64) -> Self {
        match &mut self.kind {
            Kind::Str { max, .. } => *max = Some(n as usize),
            Kind::Int { max, .. } => *max = Some(n),
            Kind::Num { max, .. } => *max = Some(n as f64),
            Kind::Array { max, .. } => *max = Some(n as usize),
            _ => {}
        }
        self
    }
    pub fn url(mut self) -> Self {
        if let Kind::Str { url, .. } = &mut self.kind { *url = true; }
        self
    }
    pub fn trim(mut self) -> Self {
        if let Kind::Str { trim, .. } = &mut self.kind { *trim = true; }
        self
    }
    pub fn pattern(mut self, re: &str) -> Self {
        if let Kind::Str { pattern, .. } = &mut self.kind { *pattern = Some(regex::Regex::new(re).expect("regex")); }
        self
    }
    /// Objet qui garde les clés inconnues (`.loose()` de zod).
    pub fn loose(mut self) -> Self {
        if let Kind::Object { loose, .. } = &mut self.kind { *loose = true; }
        self
    }
    /// Ajoute des champs à un objet (`.extend()`).
    pub fn extend(mut self, extra: Vec<(&str, S)>) -> Self {
        if let Kind::Object { fields, .. } = &mut self.kind {
            for (k, v) in extra { fields.push((k.to_string(), v)); }
        }
        self
    }
    /// Champs d'un objet.
    pub fn fields(&self) -> &[(String, S)] {
        match &self.kind { Kind::Object { fields, .. } => fields, _ => &[] }
    }
    pub fn field(&self, name: &str) -> Option<&S> {
        self.fields().iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// Valide `value` ; renvoie la valeur normalisée (défauts, clés inconnues retirées).
    pub fn parse(&self, value: &Value) -> Result<Value, Vec<Issue>> {
        let mut issues = Vec::new();
        let out = self.check(Some(value), &mut Vec::new(), &mut issues);
        if issues.is_empty() { Ok(out.unwrap_or(Value::Null)) } else { Err(issues) }
    }

    /// Comme `parse`, puis désérialise vers un type Rust.
    pub fn parse_into<T: serde::de::DeserializeOwned>(&self, value: &Value) -> Result<T, Vec<Issue>> {
        let v = self.parse(value)?;
        serde_json::from_value(v).map_err(|e| vec![Issue { path: vec![], message: e.to_string() }])
    }

    fn check(&self, value: Option<&Value>, path: &mut Vec<Seg>, issues: &mut Vec<Issue>) -> Option<Value> {
        let value = match value {
            None => {
                if let Default_::Value(d) = &self.default {
                    // Comme zod 4 : le défaut est la valeur de sortie (non revalidée), sauf objets
                    // dont les champs ont leurs propres défauts.
                    return Some(self.complete_default(d));
                }
                if self.optional { return None; }
                issues.push(Issue { path: path.clone(), message: invalid_type(&self.kind, None) });
                return None;
            }
            Some(Value::Null) if self.nullable => return Some(Value::Null),
            Some(v) => v,
        };
        let before = issues.len();
        let out = self.check_kind(value, path, issues);
        if issues.len() == before {
            if let (Some(refine), Some(v)) = (&self.refine, &out) {
                if let Some(message) = refine(v) {
                    issues.push(Issue { path: path.clone(), message });
                    return out;
                }
            }
            if let (Some(t), Some(v)) = (&self.transform, out.clone()) {
                return Some(t(v));
            }
        }
        out
    }

    fn complete_default(&self, d: &Value) -> Value {
        if let (Kind::Object { .. }, Value::Object(_)) = (&self.kind, d) {
            let mut issues = Vec::new();
            if let Some(v) = self.check_kind(d, &mut Vec::new(), &mut issues) {
                if issues.is_empty() { return v; }
            }
        }
        d.clone()
    }

    fn check_kind(&self, value: &Value, path: &mut Vec<Seg>, issues: &mut Vec<Issue>) -> Option<Value> {
        let here = |issues: &mut Vec<Issue>, path: &Vec<Seg>, message: String| {
            issues.push(Issue { path: path.clone(), message })
        };
        match &self.kind {
            Kind::Any => Some(value.clone()),
            Kind::Str { min, max, url, pattern, trim } => {
                let Some(text) = value.as_str() else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    return None;
                };
                let text = if *trim { text.trim() } else { text };
                let len = text.chars().count();
                if let Some(min) = min {
                    if len < *min {
                        here(issues, path, if *min == 1 && len == 0 {
                            "Trop petit : chaîne doit avoir >=1 caractères".into()
                        } else {
                            format!("Trop petit : chaîne doit avoir >={min} caractères")
                        });
                        return None;
                    }
                }
                if let Some(max) = max {
                    if len > *max {
                        here(issues, path, format!("Trop grand : chaîne doit avoir <={max} caractères"));
                        return None;
                    }
                }
                if *url && !is_url(text) {
                    here(issues, path, "URL invalide".into());
                    return None;
                }
                if let Some(re) = pattern {
                    if !re.is_match(text) {
                        here(issues, path, format!("Chaîne invalide : doit correspondre au modèle {}", re.as_str()));
                        return None;
                    }
                }
                Some(Value::String(text.to_string()))
            }
            Kind::Int { min, max } => {
                let n = match value {
                    Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15).map(|f| f as i64)),
                    _ => None,
                };
                let Some(n) = n else {
                    let message = if value.is_number() {
                        "Entrée invalide : entier attendu, nombre reçu".to_string()
                    } else {
                        invalid_type(&self.kind, Some(value))
                    };
                    here(issues, path, message);
                    return None;
                };
                if let Some(min) = min {
                    if n < *min { here(issues, path, format!("Trop petit : nombre doit être >={min}")); return None; }
                }
                if let Some(max) = max {
                    if n > *max { here(issues, path, format!("Trop grand : nombre doit être <={max}")); return None; }
                }
                Some(json!(n))
            }
            Kind::Num { min, max } => {
                let Some(n) = value.as_f64() else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    return None;
                };
                if let Some(min) = min {
                    if n < *min { here(issues, path, format!("Trop petit : nombre doit être >={min}")); return None; }
                }
                if let Some(max) = max {
                    if n > *max { here(issues, path, format!("Trop grand : nombre doit être <={max}")); return None; }
                }
                Some(value.clone())
            }
            Kind::Bool => {
                if value.is_boolean() { Some(value.clone()) } else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    None
                }
            }
            Kind::Enum(values) => match value.as_str() {
                Some(v) if values.iter().any(|x| x == v) => Some(value.clone()),
                _ => {
                    here(issues, path, format!(
                        "Option invalide : une valeur parmi {} attendue",
                        values.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join("|")
                    ));
                    None
                }
            },
            Kind::Literal(expected) => {
                if value == expected { Some(value.clone()) } else {
                    here(issues, path, format!("Entrée invalide : {} attendu", expected));
                    None
                }
            }
            Kind::Array { item, min, max } => {
                let Some(items) = value.as_array() else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    return None;
                };
                if let Some(min) = min {
                    if items.len() < *min {
                        here(issues, path, format!("Trop petit : tableau doit avoir >={min} éléments"));
                        return None;
                    }
                }
                if let Some(max) = max {
                    if items.len() > *max {
                        here(issues, path, format!("Trop grand : tableau doit avoir <={max} éléments"));
                        return None;
                    }
                }
                let mut out = Vec::with_capacity(items.len());
                for (i, v) in items.iter().enumerate() {
                    path.push(Seg::Index(i));
                    if let Some(v) = item.check(Some(v), path, issues) { out.push(v); }
                    path.pop();
                }
                Some(Value::Array(out))
            }
            Kind::Tuple { items, rest } => {
                let Some(values) = value.as_array() else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    return None;
                };
                if values.len() < items.len() || (rest.is_none() && values.len() > items.len()) {
                    here(issues, path, format!("Trop {} : tableau doit avoir {} éléments",
                        if values.len() < items.len() { "petit" } else { "grand" }, items.len()));
                    return None;
                }
                let mut out = Vec::new();
                for (i, v) in values.iter().enumerate() {
                    let schema = items.get(i).or(rest.as_deref()).unwrap();
                    path.push(Seg::Index(i));
                    if let Some(v) = schema.check(Some(v), path, issues) { out.push(v); }
                    path.pop();
                }
                Some(Value::Array(out))
            }
            Kind::Object { fields, loose } => {
                let Some(map) = value.as_object() else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    return None;
                };
                let mut out = Map::new();
                for (name, schema) in fields {
                    path.push(Seg::Key(name.clone()));
                    // `undefined` explicite : absent.
                    if let Some(v) = schema.check(map.get(name), path, issues) {
                        out.insert(name.clone(), v);
                    }
                    path.pop();
                }
                if *loose {
                    for (k, v) in map {
                        if !fields.iter().any(|(name, _)| name == k) { out.insert(k.clone(), v.clone()); }
                    }
                }
                Some(Value::Object(out))
            }
            Kind::Record(schema) => {
                let Some(map) = value.as_object() else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    return None;
                };
                let mut out = Map::new();
                for (k, v) in map {
                    path.push(Seg::Key(k.clone()));
                    if let Some(v) = schema.check(Some(v), path, issues) { out.insert(k.clone(), v); }
                    path.pop();
                }
                Some(Value::Object(out))
            }
            Kind::Disc { key, variants } => {
                let Some(map) = value.as_object() else {
                    here(issues, path, invalid_type(&self.kind, Some(value)));
                    return None;
                };
                let tag = map.get(key).and_then(|v| v.as_str());
                match variants.iter().find(|(name, _)| Some(name.as_str()) == tag) {
                    Some((_, schema)) => schema.check(Some(value), path, issues),
                    None => {
                        path.push(Seg::Key(key.clone()));
                        here(issues, path, format!(
                            "Option invalide : une valeur parmi {} attendue",
                            variants.iter().map(|(v, _)| format!("\"{v}\"")).collect::<Vec<_>>().join("|")
                        ));
                        path.pop();
                        None
                    }
                }
            }
            Kind::Union(variants) => {
                for variant in variants {
                    let mut local = Vec::new();
                    if let Some(v) = variant.check(Some(value), path, &mut local) {
                        if local.is_empty() { return Some(v); }
                    }
                }
                here(issues, path, "Entrée invalide".into());
                None
            }
        }
    }

    /// JSON Schema (draft 07) de cette description (entrée des outils MCP).
    pub fn json_schema(&self) -> Value {
        let mut schema = match &self.kind {
            Kind::Any => json!({}),
            Kind::Str { min, max, url, pattern, .. } => {
                let mut s = json!({ "type": "string" });
                if let Some(m) = min { s["minLength"] = json!(m); }
                if let Some(m) = max { s["maxLength"] = json!(m); }
                if *url { s["format"] = json!("uri"); }
                if let Some(p) = pattern { s["pattern"] = json!(p.as_str()); }
                s
            }
            Kind::Int { min, max } => {
                let mut s = json!({ "type": "integer" });
                if let Some(m) = min { s["minimum"] = json!(m); }
                if let Some(m) = max { s["maximum"] = json!(m); }
                s
            }
            Kind::Num { min, max } => {
                let mut s = json!({ "type": "number" });
                if let Some(m) = min { s["minimum"] = json!(m); }
                if let Some(m) = max { s["maximum"] = json!(m); }
                s
            }
            Kind::Bool => json!({ "type": "boolean" }),
            Kind::Enum(values) => json!({ "type": "string", "enum": values }),
            Kind::Literal(v) => {
                let t = match v { Value::Bool(_) => "boolean", Value::Number(_) => "number", _ => "string" };
                json!({ "type": t, "const": v })
            }
            Kind::Array { item, min, max } => {
                let mut s = json!({ "type": "array", "items": item.json_schema() });
                if let Some(m) = min { s["minItems"] = json!(m); }
                if let Some(m) = max { s["maxItems"] = json!(m); }
                s
            }
            Kind::Tuple { items, rest } => {
                let mut s = json!({ "type": "array", "prefixItems": items.iter().map(|i| i.json_schema()).collect::<Vec<_>>() });
                if let Some(r) = rest { s["items"] = r.json_schema(); }
                s
            }
            Kind::Object { fields, loose } => object_schema(fields, *loose),
            Kind::Record(v) => json!({ "type": "object", "propertyNames": { "type": "string" }, "additionalProperties": v.json_schema() }),
            Kind::Disc { variants, .. } => json!({ "anyOf": variants.iter().map(|(_, v)| v.json_schema()).collect::<Vec<_>>() }),
            Kind::Union(variants) => json!({ "anyOf": variants.iter().map(|v| v.json_schema()).collect::<Vec<_>>() }),
        };
        if self.nullable {
            schema = json!({ "anyOf": [schema, { "type": "null" }] });
        }
        if let Some(d) = &self.description { schema["description"] = json!(d); }
        if let Default_::Value(d) = &self.default { schema["default"] = d.clone(); }
        schema
    }

    fn is_required(&self) -> bool {
        !self.optional && matches!(self.default, Default_::None)
    }
}

fn object_schema(fields: &[(String, S)], loose: bool) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, schema) in fields {
        properties.insert(name.clone(), schema.json_schema());
        if schema.is_required() { required.push(json!(name)); }
    }
    let mut s = json!({ "type": "object", "properties": properties });
    if !required.is_empty() { s["required"] = Value::Array(required); }
    if !loose { s["additionalProperties"] = json!(false); }
    s
}

/// JSON Schema d'un ensemble de champs (entrée d'un outil MCP).
pub fn input_schema(fields: &[(String, S)]) -> Value {
    let mut s = object_schema(fields, false);
    s["$schema"] = json!("http://json-schema.org/draft-07/schema#");
    s
}

/// Nom français d'un type reçu.
fn received(value: Option<&Value>) -> &'static str {
    match value {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "booléen",
        Some(Value::Number(_)) => "nombre",
        Some(Value::String(_)) => "chaîne",
        Some(Value::Array(_)) => "tableau",
        Some(Value::Object(_)) => "objet",
    }
}

fn invalid_type(kind: &Kind, value: Option<&Value>) -> String {
    let expected = match kind {
        Kind::Str { .. } => "chaîne",
        Kind::Int { .. } => "entier",
        Kind::Num { .. } => "nombre",
        Kind::Bool => "booléen",
        Kind::Array { .. } | Kind::Tuple { .. } => "tableau",
        Kind::Object { .. } | Kind::Record(_) | Kind::Disc { .. } => "objet",
        Kind::Enum(values) => {
            return format!(
                "Option invalide : une valeur parmi {} attendue",
                values.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join("|")
            );
        }
        Kind::Literal(v) => return format!("Entrée invalide : {v} attendu"),
        Kind::Any | Kind::Union(_) => "valeur",
    };
    format!("Entrée invalide : {expected} attendu, {} reçu", received(value))
}

/// URL absolue (schéma + hôte), comme `z.url()`.
pub fn is_url(text: &str) -> bool {
    match url::Url::parse(text) {
        Ok(u) => !u.cannot_be_a_base() || u.scheme() == "mailto",
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defauts_et_cles_inconnues() {
        let schema = object(vec![
            ("a", int().default(json!(3))),
            ("b", string().optional()),
            ("c", object(vec![("d", boolean().default(json!(true)))]).default(json!({}))),
        ]);
        assert_eq!(schema.parse(&json!({ "x": 1 })).unwrap(), json!({ "a": 3, "c": { "d": true } }));
    }

    #[test]
    fn erreurs_avec_chemin() {
        let schema = object(vec![("items", array(object(vec![("url", string().url())])))]);
        let issues = schema.parse(&json!({ "items": [{ "url": "ok" }] })).unwrap_err();
        assert_eq!(issues[0].path_string(), "items.0.url");
        assert_eq!(issues[0].message, "URL invalide");
        let issues = schema.parse(&json!({ "items": 3 })).unwrap_err();
        assert_eq!(issues[0].describe(), "items : Entrée invalide : tableau attendu, nombre reçu");
    }

    #[test]
    fn entiers_bornes_et_enum() {
        assert!(int().min(1000).parse(&json!(3)).is_err());
        assert!(int().parse(&json!(3.5)).is_err());
        assert_eq!(int().parse(&json!(3.0)).unwrap(), json!(3));
        let issues = enumeration(&["a", "b"]).parse(&json!("c")).unwrap_err();
        assert!(issues[0].message.contains("\"a\"|\"b\""));
    }

    #[test]
    fn union_discriminee() {
        let schema = disc("type", vec![
            ("x", object(vec![("type", literal(json!("x"))), ("n", int())])),
            ("y", object(vec![("type", literal(json!("y")))])),
        ]);
        assert!(schema.parse(&json!({ "type": "y" })).is_ok());
        assert_eq!(schema.parse(&json!({ "type": "z" })).unwrap_err()[0].path_string(), "type");
        assert_eq!(schema.parse(&json!({ "type": "x" })).unwrap_err()[0].path_string(), "n");
    }

    #[test]
    fn json_schema() {
        let schema = input_schema(&[
            ("n".into(), int().min(1).describe("Nombre")),
            ("o".into(), string().optional()),
        ]);
        assert_eq!(schema["properties"]["n"], json!({ "type": "integer", "minimum": 1, "description": "Nombre" }));
        assert_eq!(schema["required"], json!(["n"]));
    }

    #[test]
    fn url() {
        assert!(is_url("https://rancher.intra"));
        assert!(is_url("http://127.0.0.1:8081"));
        assert!(!is_url("pas une url"));
        assert!(!is_url("pas-une-url"));
    }
}
