//! Mémoire de contexte sur disque : un fichier Markdown par entité, un index plein-texte en
//! mémoire (sans vecteurs) et un repli des gros fichiers dans `archive/` (jamais de perte).

use super::entity::{ContextEvent, EntityHead, parse_entity_file, render_entity_file, render_event_block};
use crate::text::fold;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ROLLOVER_BYTES: usize = 256 * 1024;
const SNIPPET: usize = 240;

const AGENTS_MD: &str = "# Contexte Ocekour

Ce répertoire est la **mémoire de contexte** tenue par Ocekour : messages Rocket.Chat reçus et
nouveautés GitLab / Jira / Confluence concernant l'utilisateur, un fichier par entité.

**N'ouvrez pas ces fichiers directement.** Interrogez-les par les outils MCP d'Ocekour :

- `contexte_chercher(motscles)` — recherche plein-texte, renvoie les extraits et références ;
- `contexte_recents(limite?)` — ce qui a bougé récemment ;
- `contexte_lire(ref)` — le contenu complet d'une entité.

Pour toute question de l'utilisateur sur une MR, un ticket, une page ou une conversation,
consultez d'abord ces outils.
";

const SOURCES: [&str; 4] = ["gitlab", "jira", "confluence", "rocketchat"];

#[derive(Debug, Clone)]
struct Indexed {
    source: String,
    title: String,
    url: Option<String>,
    maj: String,
    haystack: String,
    text: String,
}

/// Résultat de recherche renvoyé aux agents.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SearchHit {
    pub r#ref: String,
    pub source: String,
    pub titre: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub maj: String,
    pub extrait: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EntityContent {
    pub r#ref: String,
    pub contenu: String,
}

/// Nom de fichier sûr (Windows) et lisible à partir d'une clé d'entité.
pub fn safe_name(key: &str) -> String {
    let s: String = key.replace(['/', '\\'], "-").chars().filter(|c| !"<>:\"|?*".contains(*c) && (*c as u32) >= 0x20).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.trim().chars().take(120).collect();
    if s.is_empty() { "sans-nom".into() } else { s }
}

fn snippet(text: &str, term: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let take = |from: usize| clean.chars().skip(from).take(SNIPPET).collect::<String>();
    if term.is_empty() { return take(0); }
    // Position (en caractères) du terme dans le texte replié.
    let folded: Vec<char> = clean.chars().collect();
    let haystack: Vec<String> = folded.iter().map(|c| fold(&c.to_string())).collect();
    let joined: String = haystack.concat();
    let Some(byte_at) = joined.find(term) else { return take(0) };
    let mut acc = 0;
    let mut at = 0;
    for (i, h) in haystack.iter().enumerate() {
        if acc >= byte_at { at = i; break; }
        acc += h.len();
        at = i + 1;
    }
    let start = at.saturating_sub(SNIPPET / 4);
    format!("{}{}", if start > 0 { "…" } else { "" }, take(start))
}

pub struct ContextStore {
    dir: PathBuf,
    index: Mutex<Option<BTreeMap<String, Indexed>>>,
}

impl ContextStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self { ContextStore { dir: dir.into(), index: Mutex::new(None) } }

    pub fn dir(&self) -> &Path { &self.dir }

    fn with_index<T>(&self, f: impl FnOnce(&mut BTreeMap<String, Indexed>) -> T) -> T {
        let mut guard = self.index.lock();
        if guard.is_none() {
            let mut index = BTreeMap::new();
            if let Err(e) = self.load_into(&mut index) { log::warn!("Contexte : chargement du répertoire : {e}"); }
            *guard = Some(index);
        }
        f(guard.as_mut().unwrap())
    }

    fn load_into(&self, index: &mut BTreeMap<String, Indexed>) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        if !self.dir.join("AGENTS.md").exists() { self.write_atomic("AGENTS.md", AGENTS_MD)?; }
        for source in SOURCES {
            let Ok(entries) = std::fs::read_dir(self.dir.join(source)) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".md") { continue; }
                let rel = format!("{source}/{name}");
                if let Ok(text) = std::fs::read_to_string(entry.path()) { reindex(index, &rel, &text); }
            }
        }
        Ok(())
    }

    /// Charge (une fois) le contenu existant.
    pub fn load(&self) { self.with_index(|_| ()); }

    /// Verse un événement dans le fichier de son entité.
    pub fn ingest(&self, event: &ContextEvent) -> anyhow::Result<()> {
        self.with_index(|index| -> anyhow::Result<()> {
            let rel = format!("{}/{}.md", event.entity.source, safe_name(&event.entity.key));
            let existing = std::fs::read_to_string(self.dir.join(&rel)).ok();
            let (head, body) = existing.as_deref().map(parse_entity_file).unwrap_or((None, String::new()));
            let head = head.unwrap_or_default();
            let mut participants: Vec<String> = head.participants.clone().unwrap_or_default();
            for p in event.entity.participants.iter().flatten().chain(event.actor.iter()) {
                if !p.is_empty() && !participants.contains(p) { participants.push(p.clone()); }
            }
            let next_head = EntityHead {
                source: event.entity.source.clone(),
                r#type: event.entity.r#type.clone(),
                key: event.entity.key.clone(),
                titre: [event.entity.title.as_str(), head.titre.as_str(), event.entity.key.as_str()].into_iter().find(|t| !t.is_empty()).unwrap_or_default().into(),
                url: event.entity.url.clone().or(head.url),
                participants: (!participants.is_empty()).then_some(participants),
                maj: event.at.clone(),
            };
            let mut next_body = format!("{}\n\n{}\n", body.trim_end(), render_event_block(event)).trim_start_matches('\n').to_string();
            if next_body.len() > ROLLOVER_BYTES { next_body = self.rollover(&event.entity.source, &event.entity.key, next_body); }
            let rendered = render_entity_file(&next_head, &next_body);
            self.write_atomic(&rel, &rendered)?;
            reindex(index, &rel, &rendered);
            let rows: Vec<String> = recents_of(index, 50)
                .iter()
                .map(|e| format!("- [{}]({}) · {} · {}", e.titre, e.r#ref, e.source, e.maj.chars().take(16).collect::<String>().replace('T', " ")))
                .collect();
            let rows = if rows.is_empty() { "_Rien pour le moment._".to_string() } else { rows.join("\n") };
            self.write_atomic("index.md", &format!("# Contexte — récents\n\n{rows}\n"))?;
            Ok(())
        })
    }

    /// Replie l'ancien journal dans `archive/` et repart d'un corps court avec un lien.
    fn rollover(&self, source: &str, key: &str, body: String) -> String {
        let stamp = crate::text::now_iso().replace([':', '.'], "-");
        let name = format!("{}-{stamp}.md", safe_name(key));
        match self.write_atomic(&format!("{source}/archive/{name}"), &body) {
            Ok(()) => {
                log::info!("Contexte : {key} replié dans {source}/archive/{name}");
                format!("> Historique plus ancien : [archive](archive/{name})\n")
            }
            Err(e) => {
                log::warn!("Contexte : repli impossible : {e}");
                body
            }
        }
    }

    /// Recherche plein-texte (mots-clés, tous requis), les plus récents d'abord.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let terms: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
        if terms.is_empty() { return vec![]; }
        self.with_index(|index| {
            let mut hits: Vec<SearchHit> = index
                .iter()
                .filter(|(_, e)| terms.iter().all(|t| e.haystack.contains(t)))
                .map(|(rel, e)| SearchHit { r#ref: rel.clone(), source: e.source.clone(), titre: e.title.clone(), url: e.url.clone(), maj: e.maj.clone(), extrait: snippet(&e.text, &terms[0]) })
                .collect();
            hits.sort_by(|a, b| b.maj.cmp(&a.maj));
            hits.truncate(limit);
            hits
        })
    }

    /// Entités récemment touchées.
    pub fn recents(&self, limit: usize) -> Vec<SearchHit> { self.with_index(|index| recents_of(index, limit)) }

    /// Contenu complet d'une entité, par référence (`gitlab/grp-app!42.md`) ou clé.
    pub fn read(&self, r: &str) -> Option<EntityContent> {
        let rel = self.with_index(|index| {
            if index.contains_key(r) { return Some(r.to_string()); }
            let with_md = if r.ends_with(".md") { r.to_string() } else { format!("{r}.md") };
            if index.contains_key(&with_md) { return Some(with_md); }
            let safe = format!("/{}.md", safe_name(r));
            index.keys().find(|k| k.ends_with(&safe)).cloned()
        })?;
        std::fs::read_to_string(self.dir.join(&rel)).ok().map(|contenu| EntityContent { r#ref: rel, contenu })
    }

    pub fn size(&self) -> usize { self.with_index(|i| i.len()) }

    /// Écriture atomique (fichier temporaire puis renommage).
    fn write_atomic(&self, rel: &str, content: &str) -> std::io::Result<()> {
        let abs = self.dir.join(rel);
        if let Some(parent) = abs.parent() { std::fs::create_dir_all(parent)?; }
        let tmp = abs.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &abs)
    }
}

fn reindex(index: &mut BTreeMap<String, Indexed>, rel: &str, text: &str) {
    let (head, body) = parse_entity_file(text);
    let Some(head) = head.filter(|h| !h.source.is_empty() && !h.key.is_empty()) else { return };
    let title = if head.titre.is_empty() { head.key.clone() } else { head.titre.clone() };
    index.insert(rel.into(), Indexed {
        haystack: fold(&format!("{title} {} {body}", head.key)),
        source: head.source,
        title,
        url: head.url,
        maj: head.maj,
        text: body,
    });
}

fn recents_of(index: &BTreeMap<String, Indexed>, limit: usize) -> Vec<SearchHit> {
    let mut all: Vec<SearchHit> = index
        .iter()
        .map(|(rel, e)| SearchHit { r#ref: rel.clone(), source: e.source.clone(), titre: e.title.clone(), url: e.url.clone(), maj: e.maj.clone(), extrait: snippet(&e.text, "") })
        .collect();
    all.sort_by(|a, b| b.maj.cmp(&a.maj));
    all.truncate(limit);
    all
}
