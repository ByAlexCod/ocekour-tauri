//! Conversions de texte : l'agent écrit du Markdown, Jira attend du wiki-markup et Confluence
//! du « stockage » (XHTML + macros). Signature des écritures.

use fancy_regex::{Captures, Regex as FRegex};
use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignatureFormat {
    Markdown,
    Wiki,
    Storage,
}

// Jira / Confluence : sans emoji (bases MySQL « utf8 » sur 3 octets).
fn signature(format: SignatureFormat) -> &'static str {
    match format {
        SignatureFormat::Markdown => "_— 🤖 via Ocekour_",
        SignatureFormat::Wiki => "_— via Ocekour (agent IA)_",
        SignatureFormat::Storage => "<p><em>— via Ocekour (agent IA)</em></p>",
    }
}

/// Ajoute la signature Ocekour en fin de texte (une seule fois).
pub fn sign(text: &str, format: SignatureFormat) -> String {
    let sig = signature(format);
    let trimmed = text.trim_end();
    if trimmed.ends_with(sig) { return trimmed.into(); }
    if trimmed.is_empty() { return sig.into(); }
    if format == SignatureFormat::Storage { format!("{trimmed}{sig}") } else { format!("{trimmed}\n\n{sig}") }
}

fn re(s: &str) -> FRegex { FRegex::new(s).unwrap() }

fn replace_all(re: &FRegex, text: &str, mut rep: impl FnMut(&Captures<str>) -> String) -> String {
    let mut out = String::new();
    let mut last = 0;
    for c in re.captures_iter(text).flatten() {
        let m = c.get(0).unwrap();
        out.push_str(&text[last..m.start()]);
        out.push_str(&rep(&c));
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

fn g<'a>(c: &'a Captures<str>, i: usize) -> &'a str { c.get(i).map(|m| m.as_str()).unwrap_or("") }

// --- Markdown → wiki Jira --------------------------------------------------------------------

fn jira_inline(text: &str) -> String {
    static CODE: Lazy<FRegex> = Lazy::new(|| re(r"`([^`]+)`"));
    static IMAGE: Lazy<FRegex> = Lazy::new(|| re(r"!\[([^\]]*)\]\(([^)\s]+)[^)]*\)"));
    static LINK: Lazy<FRegex> = Lazy::new(|| re(r"\[([^\]]+)\]\(([^)\s]+)[^)]*\)"));
    static BOLD: Lazy<FRegex> = Lazy::new(|| re(r"(\*\*|__)(?=\S)(.+?)(?<=\S)\1"));
    static ITALIC: Lazy<FRegex> = Lazy::new(|| re(r"(^|[^*\w])\*(?=\S)([^*]+?)(?<=\S)\*(?!\w)"));
    static STRIKE: Lazy<FRegex> = Lazy::new(|| re(r"~~(?=\S)(.+?)(?<=\S)~~"));
    static PLACEHOLDER: Lazy<FRegex> = Lazy::new(|| re("\u{0}(\\d+)\u{0}"));
    let mut codes: Vec<String> = vec![];
    let mut s = replace_all(&CODE, text, |c| {
        codes.push(g(c, 1).to_string());
        format!("\u{0}{}\u{0}", codes.len() - 1)
    });
    s = replace_all(&IMAGE, &s, |c| format!("!{}!", g(c, 2)));
    s = replace_all(&LINK, &s, |c| format!("[{}|{}]", g(c, 1), g(c, 2)));
    s = replace_all(&BOLD, &s, |c| format!("\u{1}{}\u{1}", g(c, 2)));
    s = replace_all(&ITALIC, &s, |c| format!("{}_{}_", g(c, 1), g(c, 2)));
    s = replace_all(&STRIKE, &s, |c| format!("-{}-", g(c, 1)));
    s = s.replace('\u{1}', "*");
    replace_all(&PLACEHOLDER, &s, |c| format!("{{{{{}}}}}", codes[g(c, 1).parse::<usize>().unwrap()]))
}

fn table_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| jira_inline(c.trim())).collect()
}

/// Markdown (GFM courant) → wiki-markup Jira.
pub fn markdown_to_jira(markdown: &str) -> String {
    static ITEM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(\s*)([-*+]|\d+[.)])\s+(.*)$").unwrap());
    static FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(`{3,}|~{3,})\s*([\w#+.-]*)\s*$").unwrap());
    static ROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\|.*\|\s*$").unwrap());
    static SEP: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\|?\s*:?-{3,}").unwrap());
    static HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*#*\s*$").unwrap());
    static RULE: Lazy<FRegex> = Lazy::new(|| re(r"^\s*([-*_])(\s*\1){2,}\s*$"));
    static QUOTE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*>").unwrap());
    static QUOTE_PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*>\s?").unwrap());
    let text = markdown.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = vec![];
    let mut list: Vec<(usize, char)> = vec![];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let item = ITEM.captures(line);
        if item.is_none() { list.clear(); }
        if let Some(f) = FENCE.captures(line) {
            let mut body = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with(&f[1]) {
                body.push(lines[i].to_string());
                i += 1;
            }
            out.push(if f[2].is_empty() { "{code}".into() } else { format!("{{code:{}}}", &f[2]) });
            out.extend(body);
            out.push("{code}".into());
            i += 1;
            continue;
        }
        if ROW.is_match(line) && lines.get(i + 1).is_some_and(|n| SEP.is_match(n)) {
            out.push(format!("||{}||", table_cells(line).join("||")));
            i += 2;
            while i < lines.len() && ROW.is_match(lines[i]) {
                out.push(format!("|{}|", table_cells(lines[i]).join("|")));
                i += 1;
            }
            continue;
        }
        if let Some(h) = HEADING.captures(line) {
            out.push(format!("h{}. {}", h[1].len(), jira_inline(&h[2])));
            i += 1;
            continue;
        }
        if RULE.is_match(line).unwrap_or(false) {
            out.push("----".into());
            i += 1;
            continue;
        }
        if QUOTE.is_match(line) {
            let mut quote = vec![];
            while i < lines.len() && QUOTE.is_match(lines[i]) {
                quote.push(jira_inline(&QUOTE_PREFIX.replace(lines[i], "")));
                i += 1;
            }
            out.push("{quote}".into());
            out.extend(quote);
            out.push("{quote}".into());
            continue;
        }
        if let Some(it) = item {
            let indent = it[1].replace('\t', "    ").len();
            let kind = if it[2].chars().next().unwrap().is_ascii_digit() { '#' } else { '*' };
            while list.last().is_some_and(|l| l.0 > indent) { list.pop(); }
            if list.last().is_none_or(|l| l.0 < indent) { list.push((indent, kind)); } else if let Some(l) = list.last_mut() { l.1 = kind; }
            let prefix: String = list.iter().map(|l| l.1).collect();
            out.push(format!("{prefix} {}", jira_inline(&it[3])));
            i += 1;
            continue;
        }
        out.push(jira_inline(line));
        i += 1;
    }
    out.join("\n")
}

// --- Markdown → stockage Confluence ----------------------------------------------------------

fn confluence_language(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "javascript" | "js" | "typescript" | "ts" | "json" | "jsx" | "tsx" => "js",
        "python" | "py" => "py",
        "sh" | "shell" | "bash" | "zsh" => "bash",
        "yaml" | "yml" => "yml",
        "html" | "xml" => "xml",
        "java" | "kotlin" => "java",
        "groovy" => "groovy",
        "csharp" | "c#" | "cs" => "c#",
        "cpp" | "c" => "cpp",
        "css" => "css",
        "sass" | "scss" => "sass",
        "sql" => "sql",
        "php" => "php",
        "perl" => "perl",
        "ruby" => "ruby",
        "scala" => "scala",
        "powershell" | "ps1" => "powershell",
        "diff" => "diff",
        "vb" => "vb",
        "erlang" => "erl",
        _ => return None,
    })
}

fn unescape_html(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&#x27;", "'").replace("&amp;", "&")
}

fn escape_html(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }

fn cdata(s: &str) -> String { format!("<![CDATA[{}]]>", s.replace("]]>", "]]]]><![CDATA[>")) }

/// Markdown → format de stockage Confluence (XHTML, blocs de code en macro « code »).
pub fn markdown_to_storage(markdown: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut html_out = String::new();
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    html::push_html(&mut html_out, Parser::new_ext(markdown, opts));
    static CODE: Lazy<FRegex> = Lazy::new(|| re(r#"<pre><code(?: class="language-([^"]+)")?>([\s\S]*?)</code></pre>"#));
    static CHECKED: Lazy<Regex> = Lazy::new(|| Regex::new(r"<input[^>]*checked[^>]*>").unwrap());
    static INPUT: Lazy<Regex> = Lazy::new(|| Regex::new(r"<input[^>]*>").unwrap());
    static VOID: Lazy<FRegex> = Lazy::new(|| re(r"<(br|hr|img)\b([^>]*?)\s*/?>"));
    let s = replace_all(&CODE, &html_out, |c| {
        let lang = c.get(1).and_then(|l| confluence_language(&l.as_str().to_lowercase()));
        let code = unescape_html(g(c, 2));
        let code = code.strip_suffix('\n').unwrap_or(&code);
        format!(
            "<ac:structured-macro ac:name=\"code\">{}<ac:plain-text-body>{}</ac:plain-text-body></ac:structured-macro>",
            lang.map(|l| format!("<ac:parameter ac:name=\"language\">{l}</ac:parameter>")).unwrap_or_default(),
            cdata(code)
        )
    });
    let s = CHECKED.replace_all(&s, "☑");
    let s = INPUT.replace_all(&s, "☐");
    replace_all(&VOID, &s, |c| format!("<{}{} />", g(c, 1), g(c, 2))).trim().to_string()
}

// --- Stockage Confluence → Markdown ----------------------------------------------------------

fn attr(tag: &str, name: &str) -> String {
    Regex::new(&format!(r#"{}="([^"]*)""#, regex::escape(name))).unwrap().captures(tag).map(|c| c[1].to_string()).unwrap_or_default()
}

fn cdata_text(s: &str) -> String {
    static CD: Lazy<Regex> = Lazy::new(|| Regex::new(r"<!\[CDATA\[([\s\S]*?)\]\]>").unwrap());
    CD.replace_all(s, |c: &regex::Captures| escape_html(&c[1])).into_owned()
}

fn panel(name: &str) -> Option<&'static str> {
    Some(match name {
        "info" => "Info",
        "note" => "Note",
        "warning" => "Attention",
        "tip" => "Astuce",
        "panel" | "expand" => "",
        _ => return None,
    })
}

/// Macros, liens et images Confluence → HTML simple.
fn simplify_storage(storage: &str) -> String {
    static MACRO: Lazy<FRegex> = Lazy::new(|| re(r"<ac:structured-macro\b([^>]*)>((?:(?!<ac:structured-macro\b)[\s\S])*?)</ac:structured-macro>"));
    static PLAIN: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:plain-text-body>([\s\S]*?)</ac:plain-text-body>").unwrap());
    static RICH: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:rich-text-body>([\s\S]*?)</ac:rich-text-body>").unwrap());
    static CD: Lazy<Regex> = Lazy::new(|| Regex::new(r"<!\[CDATA\[([\s\S]*?)\]\]>").unwrap());
    static TAGS: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").unwrap());
    let mut s = storage.to_string();
    for _ in 0..50 {
        if !MACRO.is_match(&s).unwrap_or(false) { break; }
        s = replace_all(&MACRO, &s, |c| {
            let name = attr(g(c, 1), "ac:name");
            let inner = g(c, 2);
            let param = |p: &str| {
                Regex::new(&format!(r#"<ac:parameter ac:name="{}">([\s\S]*?)</ac:parameter>"#, regex::escape(p))).unwrap().captures(inner).map(|m| m[1].to_string())
            };
            let plain = PLAIN.captures(inner).map(|m| m[1].to_string());
            let rich = RICH.captures(inner).map(|m| m[1].to_string());
            if name == "code" || name == "noformat" {
                let lang = param("language");
                let code = match &plain {
                    Some(p) => CD.replace_all(p, "$1").into_owned(),
                    None => unescape_html(&TAGS.replace_all(inner, "")),
                };
                return format!(
                    "<pre><code{}>{}</code></pre>",
                    lang.map(|l| format!(" class=\"language-{l}\"")).unwrap_or_default(),
                    escape_html(&code)
                );
            }
            if panel(&name).is_some() || rich.is_some() {
                let title = param("title").unwrap_or_else(|| panel(&name).unwrap_or_default().to_string());
                let body = rich.unwrap_or_default();
                return if panel(&name).is_some() && name != "expand" {
                    format!("<blockquote>{}{body}</blockquote>", if title.is_empty() { String::new() } else { format!("<p><strong>{title} :</strong></p>") })
                } else {
                    format!("{}{body}", if title.is_empty() { String::new() } else { format!("<p><strong>{title}</strong></p>") })
                };
            }
            if let Some(p) = plain { return format!("<pre><code>{}</code></pre>", cdata_text(&p)); }
            format!("<p>[macro {name}]</p>")
        });
    }
    static LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:link\b[^>]*>([\s\S]*?)</ac:link>").unwrap());
    static LINK_BODY: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:plain-text-link-body>([\s\S]*?)</ac:plain-text-link-body>|<ac:link-body>([\s\S]*?)</ac:link-body>").unwrap());
    static TARGET: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ri:(page|user|attachment|space)\b([^>]*)/?>").unwrap());
    static IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:image\b[^>]*>([\s\S]*?)</ac:image>").unwrap());
    static IMG_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<ri:url\b[^>]*ri:value="([^"]*)""#).unwrap());
    static IMG_FILE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<ri:attachment\b[^>]*ri:filename="([^"]*)""#).unwrap());
    static TASK: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:task>([\s\S]*?)</ac:task>").unwrap());
    static TASK_BODY: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:task-body>([\s\S]*?)</ac:task-body>").unwrap());
    static EMOTICON: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:emoticon\b[^>]*/?>").unwrap());
    static PARAM: Lazy<Regex> = Lazy::new(|| Regex::new(r"<ac:parameter\b[^>]*>[\s\S]*?</ac:parameter>").unwrap());
    static NS: Lazy<Regex> = Lazy::new(|| Regex::new(r"</?(ac|ri):[^>]*>").unwrap());
    let s = LINK.replace_all(&s, |c: &regex::Captures| {
        let inner = &c[1];
        let body = LINK_BODY.captures(inner).and_then(|m| m.get(1).or(m.get(2)).map(|x| x.as_str().to_string()));
        let target = TARGET.captures(inner);
        let label = match (&body, &target) {
            (Some(b), _) => TAGS.replace_all(&cdata_text(b), "").into_owned(),
            (None, Some(t)) => ["ri:content-title", "ri:filename", "ri:username", "ri:userkey", "ri:space-key"]
                .iter()
                .map(|a| attr(&t[2], a))
                .find(|v| !v.is_empty())
                .unwrap_or_default(),
            _ => String::new(),
        };
        if target.as_ref().is_some_and(|t| &t[1] == "user") { format!("@{label}") } else { format!("<em>{label}</em>") }
    });
    let s = IMAGE.replace_all(&s, |c: &regex::Captures| {
        let inner = &c[1];
        match IMG_URL.captures(inner) {
            Some(u) => format!("<img src=\"{}\" alt=\"\" />", &u[1]),
            None => format!("[image : {}]", IMG_FILE.captures(inner).map(|f| f[1].to_string()).unwrap_or_else(|| "?".into())),
        }
    });
    let s = TASK.replace_all(&s, |c: &regex::Captures| {
        let inner = &c[1];
        let done = inner.contains("<ac:task-status>complete</ac:task-status>");
        let body = TASK_BODY.captures(inner).map(|b| b[1].to_string()).unwrap_or_default();
        format!("<li>{} {body}</li>", if done { "[x]" } else { "[ ]" })
    });
    let s = s.replace("<ac:task-list>", "<ul>").replace("</ac:task-list>", "</ul>");
    let s = EMOTICON.replace_all(&s, "");
    let s = PARAM.replace_all(&s, "");
    let s = NS.replace_all(&s, "");
    CD.replace_all(&s, |c: &regex::Captures| escape_html(&c[1])).into_owned()
}

/// Convertisseur HTML → Markdown (style turndown : titres `#`, blocs ``` , puces `-`).
pub fn html_to_markdown_converter(skip: Vec<&str>) -> htmd::HtmlToMarkdown {
    let mut builder = htmd::HtmlToMarkdown::builder();
    if !skip.is_empty() { builder = builder.skip_tags(skip); }
    // Italique `_…_` (comme turndown).
    builder = builder.add_handler(vec!["i", "em"], |h: &dyn htmd::element_handler::Handlers, e: htmd::Element| {
        let content = h.walk_children(e.node).content;
        let trimmed = content.trim();
        if trimmed.is_empty() { return None; }
        let lead = &content[..content.len() - content.trim_start().len()];
        let trail = &content[content.trim_end().len()..];
        Some(format!("{lead}_{trimmed}_{trail}").into())
    });
    builder
        .options(htmd::options::Options {
            bullet_list_marker: htmd::options::BulletListMarker::Dash,
            ..Default::default()
        })
        .build()
}

/// Format de stockage Confluence → Markdown lisible par l'agent.
pub fn storage_to_markdown(storage: &str) -> String {
    static CONVERTER: Lazy<htmd::HtmlToMarkdown> = Lazy::new(|| html_to_markdown_converter(vec![]));
    CONVERTER.convert(&simplify_storage(storage)).unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_vers_jira() {
        assert_eq!(
            markdown_to_jira(
                "## Titre\n\n**gras**, *italique*, `code`, [lien](https://x.y)\n\n- a\n  - b\n1. c\n\n```js\nx();\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n> cité"
            ),
            "h2. Titre\n\n*gras*, _italique_, {{code}}, [lien|https://x.y]\n\n* a\n** b\n# c\n\n{code:js}\nx();\n{code}\n\n||A||B||\n|1|2|\n\n{quote}\ncité\n{quote}"
        );
        assert_eq!(markdown_to_jira("Corrigé par **!12**"), "Corrigé par *!12*");
        assert_eq!(markdown_to_jira("~~barré~~ et ---"), "-barré- et ---");
    }

    #[test]
    fn markdown_et_stockage_confluence() {
        let storage = markdown_to_storage("# T\n\nligne<br>\n\n```python\nif a < b: pass\n```\n\n---\n");
        assert!(storage.contains("<h1>T</h1>"), "{storage}");
        assert!(storage.contains(
            "<ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">py</ac:parameter><ac:plain-text-body><![CDATA[if a < b: pass]]></ac:plain-text-body></ac:structured-macro>"
        ), "{storage}");
        assert!(storage.contains("<hr />"), "{storage}");
        assert!(storage_to_markdown(&storage).contains("```py\nif a < b: pass\n```"), "{}", storage_to_markdown(&storage));
        assert_eq!(markdown_to_storage("# Nouveau"), "<h1>Nouveau</h1>");
        let md = storage_to_markdown(
            r#"<ac:structured-macro ac:name="warning"><ac:rich-text-body><p>Voir <ac:link><ri:page ri:content-title="Autre" /></ac:link> et <ac:link><ri:user ri:username="bob" /></ac:link></p></ac:rich-text-body></ac:structured-macro>"#,
        );
        assert!(md.starts_with("> **Attention :**"), "{md}");
        assert!(md.contains("> Voir _Autre_ et @bob"), "{md}");
        let page = storage_to_markdown(
            r#"<h1>Déployer</h1><p>Lancer :</p><ac:structured-macro ac:name="code"><ac:parameter ac:name="language">bash</ac:parameter><ac:plain-text-body><![CDATA[make deploy ENV=prod]]></ac:plain-text-body></ac:structured-macro>"#,
        );
        assert_eq!(page, "# Déployer\n\nLancer :\n\n```bash\nmake deploy ENV=prod\n```");
    }

    #[test]
    fn signature_unique() {
        assert_eq!(sign("Bonjour", SignatureFormat::Markdown), "Bonjour\n\n_— 🤖 via Ocekour_");
        assert_eq!(sign(&sign("Bonjour", SignatureFormat::Markdown), SignatureFormat::Markdown), "Bonjour\n\n_— 🤖 via Ocekour_");
        assert_eq!(sign("x", SignatureFormat::Wiki), "x\n\n_— via Ocekour (agent IA)_");
        assert_eq!(sign("<p>x</p>", SignatureFormat::Storage), "<p>x</p><p><em>— via Ocekour (agent IA)</em></p>");
        assert_eq!(sign("", SignatureFormat::Wiki), "_— via Ocekour (agent IA)_");
    }
}
