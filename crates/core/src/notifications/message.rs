//! Message Rocket.Chat groupé (Markdown) et résumé pour la notification système.

use super::state::NotifItem;

const ORDER: [&str; 3] = ["gitlab", "jira", "confluence"];

fn source_label(source: &str) -> &'static str {
    match source { "gitlab" => "GitLab", "jira" => "Jira", "confluence" => "Confluence", _ => "Ocekour" }
}

fn kind_label(item: &NotifItem) -> String {
    match item.source.as_str() {
        "gitlab" => super::gitlab::kind_label(&item.kind),
        "jira" => super::jira::kind_label(&item.kind),
        "confluence" => super::confluence::kind_label(&item.kind),
        _ => item.kind.clone(),
    }
}

/// Crochets du titre échappés (ils casseraient le lien Markdown).
fn escape_link(text: &str) -> String { text.replace(['[', ']'], " ").trim().to_string() }

/// « • Revue demandée — [titre](url) · @auteur ».
fn line(item: &NotifItem) -> String {
    let title = match &item.url { Some(u) => format!("[{}]({u})", escape_link(&item.title)), None => escape_link(&item.title) };
    let actor = item.actor.as_ref().map(|a| format!(" · @{a}")).unwrap_or_default();
    format!("• {} — {title}{actor}", kind_label(item))
}

/// Message groupé, sources dans l'ordre ; `None` si rien à dire.
pub fn build_message(items: &[NotifItem]) -> Option<String> {
    if items.is_empty() { return None; }
    let total = items.len();
    let mut parts = vec![format!("🔔 **Ocekour — {total} nouveauté{}**", if total > 1 { "s" } else { "" })];
    for i in items.iter().filter(|i| i.source == "ocekour") { parts.push(format!("⚠️ {}", i.title)); }
    for source in ORDER {
        let group: Vec<&NotifItem> = items.iter().filter(|i| i.source == source).collect();
        if group.is_empty() { continue; }
        parts.push(String::new());
        parts.push(format!("**{}**", source_label(source)));
        parts.extend(group.into_iter().map(line));
    }
    Some(parts.join("\n"))
}

fn plain_line(item: &NotifItem) -> String {
    if item.source == "ocekour" { return format!("⚠️ {}", item.title); }
    let actor = item.actor.as_ref().map(|a| format!(" (@{a})")).unwrap_or_default();
    format!("{} — {} : {}{actor}", source_label(&item.source), kind_label(item), item.title)
}

/// Titre + corps texte (quelques lignes) pour une notification native.
pub fn plain_summary(items: &[NotifItem]) -> (String, String) {
    let n = items.len();
    let mut lines: Vec<String> = items.iter().take(5).map(plain_line).collect();
    if n > 5 { lines.push(format!("… et {} autre(s)", n - 5)); }
    (format!("Ocekour — {n} nouveauté{}", if n > 1 { "s" } else { "" }), lines.join("\n"))
}
