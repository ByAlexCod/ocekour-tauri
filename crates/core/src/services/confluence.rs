//! Outils Confluence Data Center (API REST « content »). Pages désignées par id ou URL web.

use super::http::ServiceHttp;
use super::markup::{SignatureFormat, markdown_to_storage, sign, storage_to_markdown};
use super::s;
use super::tools::{WriteResult, read, write};
use crate::mcp::{Args, Tool};
use crate::obj;
use crate::schema::{self as z, S};
use serde_json::{Value, json};

const LABEL: &str = "ocekour";
const VERSION_MESSAGE: &str = "via Ocekour (agent IA)";

/// Page : id, ou espace + titre.
#[derive(Debug, Clone, PartialEq)]
pub enum PageRef {
    Id(String),
    Title { space: String, title: String },
}

/// Id de page, ou espace + titre, à partir d'un id ou d'une URL web Confluence.
pub fn page_ref(input: &str) -> anyhow::Result<PageRef> {
    let value = input.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) { return Ok(PageRef::Id(value.into())); }
    static PAGE_ID: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"[?&]pageId=(\d+)").unwrap());
    static PAGES: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"/pages/(\d+)(?:/|$|\?|#)").unwrap());
    static DISPLAY: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"/display/([^/?#]+)/([^?#]+)").unwrap());
    if let Some(c) = PAGE_ID.captures(value).or_else(|| PAGES.captures(value)) { return Ok(PageRef::Id(c[1].into())); }
    if let Some(c) = DISPLAY.captures(value) {
        let decode = |s: &str| percent_encoding::percent_decode_str(s).decode_utf8_lossy().to_string();
        return Ok(PageRef::Title { space: decode(&c[1]), title: decode(&c[2].replace('+', " ")) });
    }
    anyhow::bail!("Page Confluence illisible : {input} (id ou URL de la page).")
}

fn page_s() -> S { z::string().min(1).describe("Id de la page, ou son URL web") }
fn format_s() -> S { z::enumeration(&["markdown", "stockage"]).optional().describe("markdown (défaut) ou stockage (XHTML Confluence tel quel)") }

fn storage(value: &str, format: Option<String>) -> String {
    if format.as_deref() == Some("stockage") { value.into() } else { markdown_to_storage(value) }
}

fn web_of(http: &ServiceHttp, c: &Value) -> String {
    match c["_links"]["webui"].as_str() {
        Some(w) => http.web(w),
        None => http.web(&format!("pages/viewpage.action?pageId={}", s(&c["id"]))),
    }
}

fn brief(http: &ServiceHttp, c: &Value) -> Value {
    obj! {
        "id" => c["id"], "titre" => c["title"], "type" => c["type"], "espace" => c["space"]["key"],
        "version" => c["version"]["number"], "maj" => c["version"]["when"], "url" => web_of(http, c),
    }
}

fn arr(v: &Value) -> &[Value] { v.as_array().map(|a| a.as_slice()).unwrap_or_default() }
fn opt(a: &Value, k: &str) -> Value { a.get(k).cloned().unwrap_or(Value::Null) }

/// Page (id) à partir d'un id, d'une URL ou d'espace + titre.
async fn resolve(http: &ServiceHttp, input: &str) -> anyhow::Result<String> {
    match page_ref(input)? {
        PageRef::Id(id) => Ok(id),
        PageRef::Title { space, title } => {
            let found = http.get("rest/api/content", vec![("spaceKey", json!(space)), ("title", json!(title))]).await?;
            arr(&found["results"]).first().map(|p| s(&p["id"])).ok_or_else(|| anyhow::anyhow!("Page « {title} » introuvable dans l'espace {space}."))
        }
    }
}

async fn add_label(http: &ServiceHttp, id: &str) {
    let _ = http.post(&format!("rest/api/content/{id}/label"), json!([{ "prefix": "global", "name": LABEL }])).await;
}

pub fn tools() -> Vec<Tool> {
    const C: &str = "confluence";
    let mut t = vec![];

    t.push(read(C, "confluence_chercher", "Chercher (CQL)",
        "Recherche CQL (ex. « space = DEV AND title ~ \"déploiement\" ») ou texte simple (cherché dans le contenu des pages).",
        vec![
            ("requete", z::string().min(1).describe("CQL, ou texte simple")),
            ("espace", z::string().optional().describe("Clé d'espace (texte simple seulement)")),
            ("debut", z::int().min(0).optional()),
            ("max", z::int().min(1).max(100).optional().describe("25 par défaut")),
        ],
        |a, http| async move {
            static CQL: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"(?i)(=|~|\bin\b|\border by\b)").unwrap());
            let requete = a.string("requete");
            let cql = if CQL.is_match(&requete) {
                requete
            } else {
                let text: String = requete.chars().map(|c| if c == '"' || c == '\\' { ' ' } else { c }).collect();
                format!("text ~ \"{text}\" AND type = page{}", a.str("espace").map(|e| format!(" AND space = \"{e}\"")).unwrap_or_default())
            };
            let r = http.get("rest/api/content/search", vec![("cql", json!(cql)), ("start", opt(&a, "debut")), ("limit", json!(a.int("max").unwrap_or(25))), ("expand", json!("space,version"))]).await?;
            Ok(obj! {
                "cql" => cql, "total" => if r["totalSize"].is_null() { r["size"].clone() } else { r["totalSize"].clone() },
                "resultats" => arr(&r["results"]).iter().map(|c| brief(&http, c)).collect::<Vec<_>>(),
            })
        }));

    t.push(read(C, "confluence_page_lire", "Lire une page",
        "Page en Markdown (ou format de stockage), avec version (nécessaire pour la modifier), espace, parents et libellés.",
        vec![("page", page_s()), ("format", format_s())],
        |a, http| async move {
            let id = resolve(&http, &a.string("page")).await?;
            let c = http.get(&format!("rest/api/content/{id}"), vec![("expand", json!("body.storage,version,space,ancestors,metadata.labels"))]).await?;
            let body = s(&c["body"]["storage"]["value"]);
            let mut out = brief(&http, &c);
            let extra = obj! {
                "auteur" => c["version"]["by"]["displayName"],
                "parents" => arr(&c["ancestors"]).iter().map(|p| json!({ "id": p["id"], "titre": p["title"] })).collect::<Vec<_>>(),
                "libelles" => arr(&c["metadata"]["labels"]["results"]).iter().map(|l| l["name"].clone()).collect::<Vec<_>>(),
                "contenu" => if a.str("format").as_deref() == Some("stockage") { body } else { storage_to_markdown(&body) },
            };
            out.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            Ok(out)
        }));

    t.push(read(C, "confluence_espace", "Arborescence d’un espace", "Pages racines d'un espace, ou enfants d'une page (parent).",
        vec![
            ("espace", z::string().optional().describe("Clé de l'espace (pages racines)")),
            ("parent", page_s().optional().describe("Page dont on veut les enfants")),
            ("debut", z::int().min(0).optional()),
            ("max", z::int().min(1).max(200).optional()),
        ],
        |a, http| async move {
            let base = vec![("start", opt(&a, "debut")), ("limit", json!(a.int("max").unwrap_or(100))), ("expand", json!("children.page,version"))];
            let results = if let Some(parent) = a.str("parent") {
                let id = resolve(&http, &parent).await?;
                http.get(&format!("rest/api/content/{id}/child/page"), base).await?["results"].clone()
            } else if let Some(space) = a.str("espace") {
                let mut q = base;
                q.push(("depth", json!("root")));
                http.get(&format!("rest/api/space/{}/content/page", crate::rancher::enc(&space)), q).await?["results"].clone()
            } else {
                let spaces = http.get("rest/api/space", vec![("limit", json!(a.int("max").unwrap_or(100))), ("start", opt(&a, "debut"))]).await?;
                return Ok(Value::Array(arr(&spaces["results"]).iter().map(|sp| obj! { "cle" => sp["key"], "nom" => sp["name"], "type" => sp["type"] }).collect()));
            };
            Ok(Value::Array(arr(&results).iter().map(|c| {
                let mut v = brief(&http, c);
                if !c["children"]["page"]["size"].is_null() { v.as_object_mut().unwrap().insert("enfants".into(), c["children"]["page"]["size"].clone()); }
                v
            }).collect()))
        }));

    t.push(read(C, "confluence_pieces_jointes", "Pièces jointes",
        "Pièces jointes d’une page ; avec « fichier », renvoie son contenu s’il est textuel (1 Mo au plus).",
        vec![("page", page_s()), ("fichier", z::string().optional().describe("Nom du fichier à lire"))],
        |a, http| async move {
            let id = resolve(&http, &a.string("page")).await?;
            let r = http.get(&format!("rest/api/content/{id}/child/attachment"), vec![("limit", json!(200))]).await?;
            let list: Vec<Value> = arr(&r["results"]).iter().map(|p| obj! {
                "nom" => p["title"],
                "type" => if p["extensions"]["mediaType"].is_null() { p["metadata"]["mediaType"].clone() } else { p["extensions"]["mediaType"].clone() },
                "taille" => p["extensions"]["fileSize"],
                "url" => http.web(p["_links"]["download"].as_str().unwrap_or_default()),
            }).collect();
            let Some(name) = a.str("fichier") else { return Ok(Value::Array(list)) };
            let Some(file) = list.iter().find(|p| p["nom"] == json!(name)) else {
                anyhow::bail!("{name} absent. Pièces jointes : {}.", list.iter().map(|p| s(&p["nom"])).collect::<Vec<_>>().join(", "));
            };
            if file["taille"].as_f64().unwrap_or(0.0) > 1_000_000.0 { anyhow::bail!("{name} dépasse 1 Mo."); }
            static TEXT: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"^text/|json|xml|yaml|csv|javascript|sql").unwrap());
            if let Some(kind) = file["type"].as_str() {
                if !TEXT.is_match(kind) { anyhow::bail!("{name} n'est pas un fichier texte ({kind})."); }
            }
            Ok(Value::String(http.get_text(&s(&file["url"]), vec![]).await?))
        }));

    t.push(write(C, "confluence_page_creer", "Créer une page", "Crée une page dans un espace, sous un parent éventuel. Libellé « ocekour » ajouté.",
        vec![
            ("espace", z::string().min(1).describe("Clé de l'espace")),
            ("titre", z::string().min(1)),
            ("contenu", z::string()),
            ("format", format_s()),
            ("parent", page_s().optional()),
        ],
        |a| Ok(format!("{} : {}", a.string("espace"), a.string("titre").chars().take(80).collect::<String>())),
        false,
        |a, http| async move {
            let parent = match a.str("parent") { Some(p) => Some(resolve(&http, &p).await?), None => None };
            let mut body = json!({
                "type": "page", "title": a["titre"], "space": { "key": a["espace"] },
                "body": { "storage": { "value": storage(&a.string("contenu"), a.str("format")), "representation": "storage" } },
            });
            if let Some(p) = parent { body["ancestors"] = json!([{ "id": p }]); }
            let c = http.post("rest/api/content", body).await?;
            add_label(&http, &s(&c["id"])).await;
            let url = web_of(&http, &c);
            let mut value = brief(&http, &c);
            value["url"] = json!(url);
            Ok(WriteResult { value, url: Some(url) })
        }));

    t.push(write(C, "confluence_page_modifier", "Modifier une page",
        "Remplace le contenu (et éventuellement le titre) d’une page. « version » : celle lue avec confluence_page_lire ; refusé si la page a changé depuis. Message de version « via Ocekour » et libellé « ocekour ».",
        vec![
            ("page", page_s()),
            ("version", z::int().min(1).describe("Version lue (contrôle de modification concurrente)")),
            ("contenu", z::string()),
            ("format", format_s()),
            ("titre", z::string().optional()),
        ],
        |a| Ok(format!("page {}", a.string("page"))),
        false,
        |a, http| async move {
            let id = resolve(&http, &a.string("page")).await?;
            let current = http.get(&format!("rest/api/content/{id}"), vec![("expand", json!("version,space"))]).await?;
            let number = current["version"]["number"].as_i64();
            let read_version = a.int("version").unwrap_or(0);
            if number != Some(read_version) {
                anyhow::bail!(
                    "La page a changé depuis sa lecture (version {}, lue : {read_version}) : relisez-la avant de la modifier.",
                    number.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
                );
            }
            let c = http.put(&format!("rest/api/content/{id}"), Some(json!({
                "id": id,
                "type": if current["type"].is_null() { json!("page") } else { current["type"].clone() },
                "title": a.str("titre").map(Value::String).unwrap_or_else(|| current["title"].clone()),
                "space": { "key": current["space"]["key"] },
                "version": { "number": number.unwrap_or(0) + 1, "message": VERSION_MESSAGE },
                "body": { "storage": { "value": storage(&a.string("contenu"), a.str("format")), "representation": "storage" } },
            })), vec![]).await?;
            add_label(&http, &id).await;
            let url = web_of(&http, &c);
            let mut value = brief(&http, &c);
            value["url"] = json!(url);
            Ok(WriteResult { value, url: Some(url) })
        }));

    t.push(write(C, "confluence_commenter", "Commenter une page", "Ajoute un commentaire à une page.",
        vec![("page", page_s()), ("texte", z::string().min(1)), ("format", format_s())],
        |a| Ok(format!("page {}", a.string("page"))),
        false,
        |a, http| async move {
            let id = resolve(&http, &a.string("page")).await?;
            let c = http.post("rest/api/content", json!({
                "type": "comment", "container": { "id": id, "type": "page" },
                "body": { "storage": { "value": sign(&storage(&a.string("texte"), a.str("format")), SignatureFormat::Storage), "representation": "storage" } },
            })).await?;
            let page = http.get(&format!("rest/api/content/{id}"), vec![]).await?;
            let web = web_of(&http, &page);
            let cid = s(&c["id"]);
            let url = format!("{web}{}focusedCommentId={cid}#comment-{cid}", if web.contains('?') { '&' } else { '?' });
            Ok(WriteResult { value: json!({ "commentaire": c["id"], "url": url }), url: Some(url) })
        }));

    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_de_page() {
        assert_eq!(page_ref("123").unwrap(), PageRef::Id("123".into()));
        assert_eq!(page_ref("https://wiki.intra/pages/viewpage.action?pageId=456").unwrap(), PageRef::Id("456".into()));
        assert_eq!(page_ref("https://wiki.intra/spaces/DEV/pages/789/Titre").unwrap(), PageRef::Id("789".into()));
        assert_eq!(page_ref("https://wiki.intra/display/DEV/Mon+titre").unwrap(), PageRef::Title { space: "DEV".into(), title: "Mon titre".into() });
        assert!(page_ref("nimporte").is_err());
    }
}
