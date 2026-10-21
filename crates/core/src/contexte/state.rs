//! État de l'ingestion (`contexte-state.json`) : curseurs par source (indépendants des
//! notifications), curseurs par salon Rocket.Chat et fenêtre anti-doublon. Aucun secret.

use crate::notifications::state::Cursors;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// Rocket.Chat : dernier horodatage pris en compte par salon (rid → ISO).
pub type RoomCursors = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct StateFile {
    version: u32,
    #[serde(default)]
    initialized: bool,
    #[serde(default)]
    cursors: Cursors,
    #[serde(default)]
    rooms: RoomCursors,
    #[serde(default)]
    seen: Vec<String>,
}

const MAX_SEEN: usize = 2_000;

pub struct ContextState {
    file: PathBuf,
    data: StateFile,
    seen: HashSet<String>,
}

impl ContextState {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        let file = file.into();
        let empty = StateFile { version: 1, initialized: false, cursors: Cursors::default(), rooms: RoomCursors::new(), seen: vec![] };
        let data = match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<StateFile>(&text) {
                Ok(d) if d.version == 1 => d,
                Ok(_) => empty,
                Err(e) => {
                    log::warn!("Contexte : état illisible, réinitialisé : {e}");
                    empty
                }
            },
            Err(_) => empty,
        };
        let seen = data.seen.iter().cloned().collect();
        ContextState { file, data, seen }
    }

    pub fn initialized(&self) -> bool { self.data.initialized }
    pub fn mark_initialized(&mut self) { self.data.initialized = true; }
    pub fn cursors(&self) -> &Cursors { &self.data.cursors }
    pub fn cursors_mut(&mut self) -> &mut Cursors { &mut self.data.cursors }
    pub fn rooms(&self) -> &RoomCursors { &self.data.rooms }
    pub fn set_rooms(&mut self, rooms: RoomCursors) { self.data.rooms = rooms; }
    pub fn is_new(&self, key: &str) -> bool { !self.seen.contains(key) }

    pub fn mark_seen<'a>(&mut self, keys: impl IntoIterator<Item = &'a str>) {
        for key in keys {
            if self.seen.insert(key.to_string()) { self.data.seen.push(key.to_string()); }
        }
        if self.data.seen.len() > MAX_SEEN {
            let drop = self.data.seen.len() - MAX_SEEN;
            self.data.seen.drain(..drop);
            self.seen = self.data.seen.iter().cloned().collect();
        }
    }

    pub fn save(&self) {
        let result = (|| -> anyhow::Result<()> {
            if let Some(dir) = self.file.parent() { std::fs::create_dir_all(dir)?; }
            std::fs::write(&self.file, serde_json::to_string_pretty(&self.data)?)?;
            Ok(())
        })();
        if let Err(e) = result { log::error!("Contexte : écriture de l’état : {e}"); }
    }
}
