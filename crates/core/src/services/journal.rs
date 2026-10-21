//! Journal local des écritures faites par les agents (`actions.ndjson`, une ligne par action
//! tentée). Rotation à 10 Mo : le fichier précédent est gardé (`actions.1.ndjson`).

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Écriture faite par un agent via le MCP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentAction {
    pub id: String,
    pub date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    pub service: String,
    pub tool: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub params: Value,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Query {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub service: Option<String>,
    pub status: Option<String>,
    pub text: Option<String>,
}

type Listener = Box<dyn Fn(&AgentAction) + Send + Sync>;

pub struct ActionJournal {
    pub file: PathBuf,
    listeners: Mutex<Vec<Listener>>,
    lock: Mutex<()>,
}

impl ActionJournal {
    pub fn new(file: impl Into<PathBuf>) -> Arc<Self> {
        Arc::new(ActionJournal { file: file.into(), listeners: Mutex::new(vec![]), lock: Mutex::new(()) })
    }

    pub fn on_added(&self, listener: impl Fn(&AgentAction) + Send + Sync + 'static) { self.listeners.lock().push(Box::new(listener)); }

    fn previous(&self) -> PathBuf {
        let name = self.file.file_name().map(|n| n.to_string_lossy().replace(".ndjson", ".1.ndjson")).unwrap_or_default();
        self.file.with_file_name(name)
    }

    /// Ajoute une action (id et date attribués).
    #[allow(clippy::too_many_arguments)]
    pub fn add(&self, client: Option<String>, service: &str, tool: &str, target: &str, params: Value, url: Option<String>, status: &str, error: Option<String>) -> AgentAction {
        let action = AgentAction {
            id: uuid::Uuid::new_v4().to_string(),
            date: crate::text::now_iso(),
            client,
            service: service.into(),
            tool: tool.into(),
            target: target.into(),
            url,
            params,
            status: status.into(),
            error,
        };
        {
            let _g = self.lock.lock();
            let result = (|| -> std::io::Result<()> {
                if let Some(dir) = self.file.parent() { std::fs::create_dir_all(dir)?; }
                if std::fs::metadata(&self.file).map(|m| m.len() >= MAX_BYTES).unwrap_or(false) {
                    let _ = std::fs::remove_file(self.previous());
                    std::fs::rename(&self.file, self.previous())?;
                }
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.file)?;
                writeln!(f, "{}", serde_json::to_string(&action)?)
            })();
            if let Err(e) = result { log::error!("Journal des actions : {e}"); }
        }
        for l in self.listeners.lock().iter() { l(&action); }
        action
    }

    fn read_all(&self) -> Vec<AgentAction> {
        let mut entries = vec![];
        for file in [self.previous(), self.file.clone()] {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            // Ligne tronquée (arrêt brutal) : ignorée.
            entries.extend(text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str::<AgentAction>(l).ok()));
        }
        entries
    }

    /// Actions, la plus récente en premier, filtrées et paginées.
    pub fn list(&self, q: &Query) -> (Vec<AgentAction>, usize) {
        let text = q.text.as_deref().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty());
        let matching: Vec<AgentAction> = self
            .read_all()
            .into_iter()
            .rev()
            .filter(|a| {
                q.service.as_ref().is_none_or(|s| &a.service == s)
                    && q.status.as_ref().is_none_or(|s| &a.status == s)
                    && text.as_ref().is_none_or(|t| serde_json::to_string(a).unwrap_or_default().to_lowercase().contains(t))
            })
            .collect();
        let total = matching.len();
        let (offset, limit) = (q.offset.unwrap_or(0), q.limit.unwrap_or(100));
        (matching.into_iter().skip(offset).take(limit).collect(), total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ajoute_filtre_pagine_et_tourne() {
        let dir = tempfile::tempdir().unwrap();
        let j = ActionJournal::new(dir.path().join("actions.ndjson"));
        let seen = Arc::new(Mutex::new(0));
        let s = seen.clone();
        j.on_added(move |_| *s.lock() += 1);
        j.add(Some("claude 1".into()), "gitlab", "gitlab_mr_commenter", "grp/app !12", json!({ "texte": "x" }), Some("https://g/1".into()), "ok", None);
        j.add(None, "jira", "jira_commenter", "PAY-1", json!({}), None, "error", Some("refusé".into()));
        assert_eq!(*seen.lock(), 2);
        let (entries, total) = j.list(&Query::default());
        assert_eq!(total, 2);
        assert_eq!(entries[0].tool, "jira_commenter");
        assert_eq!(j.list(&Query { status: Some("ok".into()), ..Default::default() }).1, 1);
        assert_eq!(j.list(&Query { text: Some("PAY".into()), ..Default::default() }).0[0].service, "jira");
        assert_eq!(j.list(&Query { offset: Some(1), limit: Some(1), ..Default::default() }).0[0].service, "gitlab");
        // Rotation : le fichier courant dépasse 10 Mo.
        std::fs::OpenOptions::new().append(true).open(&j.file).unwrap().set_len(MAX_BYTES).unwrap();
        j.add(None, "gitlab", "x", "y", json!({}), None, "ok", None);
        assert!(dir.path().join("actions.1.ndjson").exists());
        assert_eq!(std::fs::read_to_string(&j.file).unwrap().lines().count(), 1);
    }
}
