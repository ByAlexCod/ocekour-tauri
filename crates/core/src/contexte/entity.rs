//! Événements de contexte et fichiers d'entité (en-tête YAML + journal Markdown horodaté).

use crate::notifications::state::{EntityRef, NotifItem};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Un événement à verser dans la mémoire de contexte, rattaché à une entité.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContextEvent {
    pub entity: EntityRef,
    pub kind: String,
    /// Horodatage ISO de l'événement.
    pub at: String,
    pub actor: Option<String>,
    /// Une ligne résumant l'événement (« @bob a commenté »).
    pub summary: String,
    pub body: Option<String>,
    pub url: Option<String>,
    pub dedup_key: String,
}

pub fn kind_verb(kind: &str) -> String {
    match kind {
        "assigned" => "assigné",
        "review_requested" => "a demandé une revue",
        "mentioned" | "directly_addressed" | "mention" => "vous a mentionné",
        "build_failed" => "pipeline en échec",
        "approval_required" => "approbation demandée",
        "unmergeable" => "MR non mergeable",
        "mr_comment" => "a commenté votre MR",
        "reply" => "a répondu à votre commentaire",
        "comment" => "a commenté",
        "status" => "a changé le statut",
        "update" => "a modifié la page",
        "message" => "a écrit",
        other => other,
    }
    .into()
}

/// Nouveauté (GitLab / Jira / Confluence) → événement ; `None` sans entité.
pub fn event_from_item(item: &NotifItem, fallback_at: &str) -> Option<ContextEvent> {
    let entity = item.entity.clone()?;
    let who = item.actor.as_ref().map(|a| format!("@{a}")).unwrap_or_else(|| "Quelqu’un".into());
    Some(ContextEvent {
        entity,
        kind: item.kind.clone(),
        at: item.at.clone().unwrap_or_else(|| fallback_at.into()),
        actor: item.actor.clone(),
        summary: format!("{who} {}", kind_verb(&item.kind)),
        body: item.body.clone(),
        url: item.url.clone(),
        dedup_key: item.dedup_key.clone(),
    })
}

/// En-tête d'un fichier d'entité.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct EntityHead {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub titre: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participants: Option<Vec<String>>,
    #[serde(default)]
    pub maj: String,
}

/// En-tête (s'il est lisible) et corps d'un fichier d'entité.
pub fn parse_entity_file(text: &str) -> (Option<EntityHead>, String) {
    let Some(rest) = text.strip_prefix("---\n") else { return (None, text.into()) };
    let Some(end) = rest.find("\n---") else { return (None, text.into()) };
    let front = &rest[..end];
    let mut body = &rest[end + 4..];
    body = body.strip_prefix('\n').unwrap_or(body);
    let head = crate::yaml::parse(front).ok().map(|d| d.to_json()).and_then(|v: Value| serde_json::from_value(v).ok());
    (head, body.into())
}

/// En-tête YAML puis corps.
pub fn render_entity_file(head: &EntityHead, body: &str) -> String {
    let front = crate::yaml::render(&serde_json::to_value(head).unwrap_or_default());
    format!("---\n{}\n---\n\n{}", front.trim_end(), body.trim_start_matches('\n'))
}

/// Bloc Markdown d'un événement (fin de journal).
pub fn render_event_block(e: &ContextEvent) -> String {
    let date: String = e.at.replace('T', " ").chars().take(16).collect();
    let mut lines = vec![format!("## {date} — {}", e.summary)];
    if let Some(body) = e.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        lines.push(body.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n"));
    }
    if let Some(url) = &e.url { lines.push(format!("[voir]({url})")); }
    lines.join("\n")
}
