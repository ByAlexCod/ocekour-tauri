//! Accès web « simple » (sans navigateur) : un GET HTTP(S) dont le corps HTML est converti en
//! Markdown lisible par l'agent. TLS comme les autres services (CA d'entreprise / auto-signé).

use crate::config::Tls;
use futures_util::StreamExt;
use std::time::Duration;

const MAX_BYTES: usize = 5_000_000;

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub url: String,
    pub status: u16,
    pub content_type: String,
    /// Corps converti : Markdown pour du HTML, texte brut sinon.
    pub body: String,
    /// Corps tronqué (trop volumineux).
    pub truncated: bool,
}

/// GET HTTP(S) brut, redirections suivies (5 au plus), borné en taille.
pub async fn http_get(raw_url: &str, tls: &Tls, user_agent: &str) -> anyhow::Result<FetchResult> {
    let mut url = url::Url::parse(raw_url).map_err(|_| anyhow::anyhow!("URL invalide : {raw_url}"))?;
    let client = crate::net::http_client(tls)?;
    let mut redirects = 5;
    loop {
        if url.scheme() != "http" && url.scheme() != "https" {
            anyhow::bail!("Seul http(s) est accepté (reçu : {}:).", url.scheme());
        }
        let res = tokio::time::timeout(
            Duration::from_secs(30),
            client
                .get(url.clone())
                .header("User-Agent", user_agent)
                .header("Accept", "text/html,text/*,application/json;q=0.9,*/*;q=0.8")
                .send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Pas de réponse après 30 s."))?
        .map_err(|e| anyhow::anyhow!(crate::net::from_reqwest(&e, url.as_str()).message))?;
        let status = res.status().as_u16();
        if (300..400).contains(&status) {
            if let Some(location) = res.headers().get("location").and_then(|l| l.to_str().ok()) {
                if redirects == 0 { anyhow::bail!("Trop de redirections."); }
                redirects -= 1;
                url = url.join(location)?;
                continue;
            }
        }
        let content_type = res.headers().get("content-type").and_then(|c| c.to_str().ok()).unwrap_or_default().to_string();
        let mut body = Vec::new();
        let mut truncated = false;
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len() + chunk.len() > MAX_BYTES {
                body.extend_from_slice(&chunk[..MAX_BYTES - body.len()]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(FetchResult { url: url.to_string(), status, content_type, body: String::from_utf8_lossy(&body).into_owned(), truncated });
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// HTML → Markdown : enlève script/style/noscript/svg/head, garde le contenu lisible.
pub fn html_to_markdown(html: &str) -> String {
    static CONVERTER: once_cell::sync::Lazy<htmd::HtmlToMarkdown> =
        once_cell::sync::Lazy::new(|| crate::services::markup::html_to_markdown_converter(vec!["script", "style", "noscript", "svg", "head"]));
    static TITLE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
    static BLANKS: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"\n{3,}").unwrap());
    let title = TITLE.captures(html).map(|c| c[1].trim().to_string()).filter(|t| !t.is_empty());
    let md = CONVERTER.convert(html).unwrap_or_default();
    let md = BLANKS.replace_all(&md, "\n\n").trim().to_string();
    match title {
        Some(t) => format!("# {}\n\n{md}", decode_entities(&t)),
        None => md,
    }
}

/// Récupère une URL et renvoie son corps en Markdown (HTML) ou en texte (autre).
pub async fn fetch_web(url: &str, tls: &Tls, user_agent: &str) -> anyhow::Result<FetchResult> {
    let mut res = http_get(url, tls, user_agent).await?;
    static HTML: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"(?i)text/html|application/xhtml").unwrap());
    if HTML.is_match(&res.content_type) { res.body = html_to_markdown(&res.body); }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::http::{Resp, handler, start};

    #[test]
    fn html_vers_markdown() {
        let md = html_to_markdown(
            "<html><head><title>Doc</title><style>body{}</style></head><body><h1>Titre</h1><p>Un <a href=\"https://x\">lien</a>.</p><script>alert(1)</script><table><tr><th>A</th></tr><tr><td>1</td></tr></table></body></html>",
        );
        assert!(md.starts_with("# Doc\n\n"), "{md}");
        assert!(md.contains("# Titre"), "{md}");
        assert!(md.contains("[lien](https://x)"), "{md}");
        assert!(!md.contains("alert") && !md.contains("body{}"), "{md}");
        assert!(md.contains("| A |"), "{md}");
    }

    #[tokio::test]
    async fn recuperation() {
        crate::net::install_default_provider();
        let server = start(0, None, handler(|r| async move {
            match r.path.as_str() {
                "/redir" => Resp::Empty(302, vec![("location".into(), "/page".into())]),
                "/boucle" => Resp::Empty(302, vec![("location".into(), "/boucle".into())]),
                "/json" => Resp::Json(200, serde_json::json!({ "a": 1 })),
                "/gros" => Resp::Text(200, "x".repeat(MAX_BYTES + 10)),
                _ => Resp::Bytes(200, b"<title>P</title><h1>Bonjour</h1>".to_vec(), vec![("content-type".into(), "text/html; charset=utf-8".into())]),
            }
        })).await;
        let tls = Tls::default();
        let res = fetch_web(&format!("{}/redir", server.url), &tls, "test").await.unwrap();
        assert_eq!(res.status, 200);
        assert!(res.url.ends_with("/page"));
        assert!(res.body.contains("# P") && res.body.contains("# Bonjour"), "{}", res.body);
        assert_eq!(fetch_web(&format!("{}/json", server.url), &tls, "test").await.unwrap().body, r#"{"a":1}"#);
        let big = fetch_web(&format!("{}/gros", server.url), &tls, "test").await.unwrap();
        assert!(big.truncated && big.body.len() == MAX_BYTES);
        assert!(fetch_web(&format!("{}/boucle", server.url), &tls, "test").await.unwrap_err().to_string().contains("redirections"));
        assert!(fetch_web("ftp://x/y", &tls, "test").await.unwrap_err().to_string().contains("http"));
    }
}
