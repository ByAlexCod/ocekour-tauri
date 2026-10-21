//! État des notifications (`notifications.json`) : curseurs par source, fenêtre anti-doublon
//! et file des nouveautés non encore envoyées. Ni configuration ni secret.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Entité que concerne une nouveauté (MR, ticket, page, fil…) : clé de regroupement de la
/// mémoire de contexte IA.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct EntityRef {
    pub source: String,
    /// merge_request | issue | ticket | page | dm | channel
    pub r#type: String,
    /// Clé stable et lisible : `grp/app!42`, `PAY-123`, un pageId, `@amartin`, `#canal`.
    pub key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participants: Option<Vec<String>>,
}

/// Une nouveauté à signaler (jamais de secret).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NotifItem {
    /// gitlab | jira | confluence | rocketchat | ocekour
    pub source: String,
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    pub dedup_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<EntityRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitlabCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SinceCursor {
    pub since: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Cursors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gitlab: Option<GitlabCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira: Option<SinceCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence: Option<SinceCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct StateFile {
    version: u32,
    #[serde(default)]
    initialized: bool,
    #[serde(default)]
    cursors: Cursors,
    #[serde(default)]
    seen: Vec<String>,
    #[serde(default)]
    pending: Vec<NotifItem>,
}

const MAX_SEEN: usize = 500;
const MAX_PENDING: usize = 50;

fn empty() -> StateFile { StateFile { version: 1, initialized: false, cursors: Cursors::default(), seen: vec![], pending: vec![] } }

pub struct NotificationState {
    file: PathBuf,
    data: StateFile,
    seen: HashSet<String>,
}

impl NotificationState {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        let file = file.into();
        let data = match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<StateFile>(&text) {
                Ok(d) if d.version == 1 => d,
                Ok(_) => empty(),
                Err(e) => {
                    log::warn!("Notifications : état illisible, réinitialisé : {e}");
                    empty()
                }
            },
            Err(_) => empty(),
        };
        let seen = data.seen.iter().cloned().collect();
        NotificationState { file, data, seen }
    }

    pub fn initialized(&self) -> bool { self.data.initialized }
    pub fn cursors(&self) -> &Cursors { &self.data.cursors }
    pub fn cursors_mut(&mut self) -> &mut Cursors { &mut self.data.cursors }

    pub fn is_new(&self, key: &str) -> bool { !self.seen.contains(key) }

    pub fn mark_seen<'a>(&mut self, keys: impl IntoIterator<Item = &'a str>) {
        for key in keys {
            if self.seen.insert(key.to_string()) { self.data.seen.push(key.to_string()); }
        }
        // Fenêtre bornée : on garde les plus récentes.
        if self.data.seen.len() > MAX_SEEN {
            let drop = self.data.seen.len() - MAX_SEEN;
            self.data.seen.drain(..drop);
            self.seen = self.data.seen.iter().cloned().collect();
        }
    }

    pub fn pending(&self) -> &[NotifItem] { &self.data.pending }

    /// Ajoute des nouveautés à la file (bornée).
    pub fn queue(&mut self, items: &[NotifItem]) {
        self.data.pending.extend_from_slice(items);
        if self.data.pending.len() > MAX_PENDING {
            let drop = self.data.pending.len() - MAX_PENDING;
            self.data.pending.drain(..drop);
        }
    }

    pub fn clear_pending(&mut self) { self.data.pending.clear(); }
    pub fn mark_initialized(&mut self) { self.data.initialized = true; }

    pub fn save(&self) {
        let result = (|| -> anyhow::Result<()> {
            if let Some(dir) = self.file.parent() { std::fs::create_dir_all(dir)?; }
            std::fs::write(&self.file, serde_json::to_string_pretty(&self.data)?)?;
            Ok(())
        })();
        if let Err(e) = result { log::error!("Notifications : écriture de l’état : {e}"); }
    }
}
