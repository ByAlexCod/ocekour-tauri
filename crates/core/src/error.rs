//! Erreurs typées des sources : l'interface propose l'action adaptée selon le code.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    Invalid,
    Unauthorized,
    Forbidden,
    Tls,
    Network,
    Timeout,
    NotRancher,
    Http,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Invalid => "invalid",
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::Forbidden => "forbidden",
            ErrorCode::Tls => "tls",
            ErrorCode::Network => "network",
            ErrorCode::Timeout => "timeout",
            ErrorCode::NotRancher => "not-rancher",
            ErrorCode::Http => "http",
        }
    }
}

/// Erreur d'une source (`{ code, message }` côté interface).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceError {
    pub code: ErrorCode,
    pub message: String,
}

impl SourceError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        SourceError { code, message: message.into() }
    }
    pub fn invalid(message: impl Into<String>) -> Self { Self::new(ErrorCode::Invalid, message) }
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.message) }
}

impl std::error::Error for SourceError {}

/// Résultat transmis à l'interface : `{ ok: true, value }` ou `{ ok: false, error }`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Outcome<T: Serialize> {
    Ok { ok: bool, value: T },
    Err { ok: bool, error: SourceError },
}

impl<T: Serialize> Outcome<T> {
    pub fn from(result: Result<T, SourceError>) -> Self {
        match result {
            Ok(value) => Outcome::Ok { ok: true, value },
            Err(error) => Outcome::Err { ok: false, error },
        }
    }
}

/// Convertit n'importe quelle erreur en `SourceError` (code `invalid` par défaut).
pub fn to_source_error(e: anyhow::Error) -> SourceError {
    match e.downcast::<SourceError>() {
        Ok(s) => s,
        Err(e) => SourceError::invalid(e.to_string()),
    }
}
