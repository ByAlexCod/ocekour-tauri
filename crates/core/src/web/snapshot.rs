//! Snapshot d'accessibilité « à la Playwright MCP » : chaque élément interactif ou porteur de
//! sens reçoit une ref stable (eN). Formatage pur ; la collecte se fait dans la page.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SnapElement {
    pub r#ref: String,
    pub role: String,
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
}

fn clip(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max { format!("{}…", flat.chars().take(max).collect::<String>()) } else { flat }
}

/// En-tête de page puis une ligne par élément (avec ref).
pub fn format_snapshot(url: &str, title: &str, elements: &[SnapElement]) -> String {
    let mut lines = vec![format!("Page : {}", clip(if title.is_empty() { "(sans titre)" } else { title }, 200)), format!("URL : {url}"), String::new()];
    if elements.is_empty() {
        lines.push("(aucun élément interactif détecté — essayez web_texte ou web_capture)".into());
        return lines.join("\n");
    }
    lines.push(format!("Éléments ({}) — cibler par [ref] :", elements.len()));
    for el in elements {
        let name = if el.name.is_empty() { String::new() } else { format!(" « {} »", clip(&el.name, 120)) };
        let value = el.value.as_deref().filter(|v| !v.is_empty()).map(|v| format!(" = « {} »", clip(v, 80))).unwrap_or_default();
        lines.push(format!("- [{}] {}{name}{value}", el.r#ref, el.role));
    }
    lines.join("\n")
}

/// Script exécuté DANS la page : marque les éléments d'un attribut data-ocekour-ref et
/// renvoie { ref, role, name, value }.
pub const COLLECT_ELEMENTS: &str = r#"(() => {
  const out = [];
  document.querySelectorAll('[data-ocekour-ref]').forEach((e) => e.removeAttribute('data-ocekour-ref'));
  const sel = 'a[href], button, input:not([type=hidden]), textarea, select, [role], summary, [onclick], ' +
    'h1, h2, h3, h4, h5, h6, [contenteditable="true"]';
  const nodes = Array.from(document.querySelectorAll(sel));
  let n = 0;
  const name = (el) => {
    const aria = el.getAttribute('aria-label');
    if (aria) return aria;
    const labelledby = el.getAttribute('aria-labelledby');
    if (labelledby) {
      const parts = labelledby.split(/\s+/).map((id) => document.getElementById(id)?.innerText ?? '').join(' ').trim();
      if (parts) return parts;
    }
    if (el.placeholder) return el.placeholder;
    if (el.getAttribute('alt')) return el.getAttribute('alt') ?? '';
    if (el.getAttribute('title')) return el.getAttribute('title') ?? '';
    const text = (el.innerText || el.textContent || '').trim();
    if (text) return text;
    if (el.value && (el.type === 'submit' || el.type === 'button')) return el.value;
    return '';
  };
  const roleOf = (el) => {
    const explicit = el.getAttribute('role');
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    if (tag === 'a') return 'link';
    if (tag === 'input') return `input:${el.type || 'text'}`;
    if (/^h[1-6]$/.test(tag)) return `heading:${tag[1]}`;
    return tag;
  };
  for (const el of nodes) {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    const visible = rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
    if (!visible) continue;
    const label = name(el).replace(/\s+/g, ' ').trim();
    const role = roleOf(el);
    const isField = el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT';
    if (!label && !isField) continue;
    const ref = `e${++n}`;
    el.setAttribute('data-ocekour-ref', ref);
    const value = isField ? el.value || undefined : undefined;
    out.push({ ref, role, name: label, value });
    if (n >= 400) break;
  }
  return out;
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_ligne_par_element() {
        let els = vec![
            SnapElement { r#ref: "e1".into(), role: "link".into(), name: "Connexion".into(), value: None },
            SnapElement { r#ref: "e2".into(), role: "input:text".into(), name: "Recherche".into(), value: Some("abc".into()) },
        ];
        let text = format_snapshot("https://x/", "Accueil", &els);
        assert!(text.contains("Page : Accueil"));
        assert!(text.contains("URL : https://x/"));
        assert!(text.contains("- [e1] link « Connexion »"));
        assert!(text.contains("- [e2] input:text « Recherche » = « abc »"));
        let empty = format_snapshot("https://x/", "", &[]);
        assert!(empty.contains("(sans titre)") && empty.contains("aucun élément interactif"));
    }
}
