//! Outils GitLab (API v4). Projets désignés par chemin (`groupe/projet`), id ou URL web.

use super::http::{Query, ServiceHttp};
use super::markup::{SignatureFormat, sign};
use super::s;
use super::tools::{WriteResult, read, write};
use crate::mcp::{Args, Tool};
use crate::obj;
use crate::schema::{self as z, S};
use serde_json::{Value, json};
use sha1::Digest;

// --- Références ------------------------------------------------------------------------------

/// Référence tirée d'une URL web GitLab (`…/groupe/projet/-/merge_requests/12`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GitlabRef {
    pub project: String,
    pub kind: Option<String>,
    pub id: Option<i64>,
}

pub fn parse_ref(base_url: &str, input: &str) -> GitlabRef {
    let value = input.trim();
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return GitlabRef { project: value.trim_matches('/').into(), ..Default::default() };
    }
    let Ok(url) = url::Url::parse(value) else { return GitlabRef { project: value.into(), ..Default::default() } };
    let base_path = url::Url::parse(base_url).map(|u| u.path().trim_end_matches('/').to_string()).unwrap_or_default();
    let mut path = percent_encoding::percent_decode_str(url.path()).decode_utf8_lossy().to_string();
    if !base_path.is_empty() && path.starts_with(&base_path) { path = path[base_path.len()..].to_string(); }
    let (project, rest) = path.split_once("/-/").unwrap_or((&path, ""));
    let mut parts = rest.split('/');
    let kind = parts.next().filter(|k| !k.is_empty()).map(str::to_string);
    let id = parts.next().and_then(|n| n.parse::<i64>().ok()).filter(|n| *n > 0);
    GitlabRef { project: project.trim_matches('/').into(), kind, id }
}

/// Retire les séquences ANSI et marqueurs de section des logs de job.
pub fn clean_job_log(text: &str) -> String {
    static SECTION: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"section_(start|end):\d+:[^\r\n]*\r(\x1b\[0K)?").unwrap());
    static ANSI: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap());
    let t = SECTION.replace_all(text, "");
    let t = ANSI.replace_all(&t, "");
    // `\r` isolé (barres de progression) → retour à la ligne.
    let mut out = String::with_capacity(t.len());
    let chars: Vec<char> = t.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == '\r' && chars.get(i + 1) != Some(&'\n') { out.push('\n'); } else { out.push(*c); }
    }
    out
}

// --- Positions de commentaire ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineType {
    New,
    Old,
    Context,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffLine {
    pub kind: LineType,
    pub old_line: i64,
    pub new_line: i64,
}

/// Lignes d'un diff unifié (numérotées comme GitLab).
pub fn parse_diff(diff: &str) -> Vec<DiffLine> {
    static HUNK: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@").unwrap());
    let (mut lines, mut in_hunk, mut old, mut new) = (vec![], false, 0i64, 0i64);
    for line in diff.split('\n') {
        if let Some(h) = HUNK.captures(line) {
            in_hunk = true;
            old = h[1].parse().unwrap_or(0);
            new = h[2].parse().unwrap_or(0);
            continue;
        }
        if !in_hunk || line.starts_with('\\') { continue; }
        if line.starts_with('+') {
            lines.push(DiffLine { kind: LineType::New, old_line: old, new_line: new });
            new += 1;
        } else if line.starts_with('-') {
            lines.push(DiffLine { kind: LineType::Old, old_line: old, new_line: new });
            old += 1;
        } else if line.starts_with(' ') {
            lines.push(DiffLine { kind: LineType::Context, old_line: old, new_line: new });
            old += 1;
            new += 1;
        }
    }
    lines
}

pub fn find_line(lines: &[DiffLine], line: i64, new_side: bool) -> Option<DiffLine> {
    lines.iter().copied().find(|l| if new_side { l.new_line == line && l.kind != LineType::Old } else { l.old_line == line && l.kind != LineType::New })
}

fn line_fields(l: &DiffLine, target: &mut serde_json::Map<String, Value>) {
    match l.kind {
        LineType::New => { target.insert("new_line".into(), json!(l.new_line)); }
        LineType::Old => { target.insert("old_line".into(), json!(l.old_line)); }
        LineType::Context => {
            target.insert("old_line".into(), json!(l.old_line));
            target.insert("new_line".into(), json!(l.new_line));
        }
    }
}

/// `line_code` GitLab : sha1(chemin)_ancienne_nouvelle.
pub fn line_code(path: &str, l: &DiffLine) -> String {
    format!("{}_{}_{}", hex::encode(sha1::Sha1::digest(path.as_bytes())), l.old_line, l.new_line)
}

/// « 3-7, 12, 20-22 ».
fn ranges(numbers: &[i64]) -> String {
    let mut sorted = numbers.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut parts = vec![];
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1] == sorted[j] + 1 { j += 1; }
        parts.push(if i == j { sorted[i].to_string() } else { format!("{}-{}", sorted[i], sorted[j]) });
        i = j + 1;
    }
    if parts.is_empty() { "aucune".into() } else { parts.join(", ") }
}

/// Position d'un commentaire de MR sur une ligne (ou une plage) du diff.
pub fn build_position(refs: &Value, file: &Value, ligne: i64, cote_ancien: bool, ligne_fin: Option<i64>) -> anyhow::Result<Value> {
    let new_path = s(&file["new_path"]);
    let lines = parse_diff(file["diff"].as_str().unwrap_or_default());
    if lines.is_empty() {
        anyhow::bail!("Pas de diff lisible pour {new_path} (fichier binaire ou diff trop gros) : faites un commentaire global.");
    }
    let new_side = !cote_ancien;
    let at = |n: i64| -> anyhow::Result<DiffLine> {
        find_line(&lines, n, new_side).ok_or_else(|| {
            let visible: Vec<i64> = lines
                .iter()
                .filter(|x| if new_side { x.kind != LineType::Old } else { x.kind != LineType::New })
                .map(|x| if new_side { x.new_line } else { x.old_line })
                .collect();
            anyhow::anyhow!(
                "Ligne {n} ({} version) hors des modifications de {new_path}. Lignes commentables : {}. Sinon, faites un commentaire global.",
                if new_side { "nouvelle" } else { "ancienne" },
                ranges(&visible)
            )
        })
    };
    let start = at(ligne)?;
    let mut position = serde_json::Map::new();
    for (k, v) in [
        ("position_type", json!("text")),
        ("base_sha", refs["base_commit_sha"].clone()),
        ("start_sha", refs["start_commit_sha"].clone()),
        ("head_sha", refs["head_commit_sha"].clone()),
        ("old_path", file["old_path"].clone()),
        ("new_path", file["new_path"].clone()),
    ] {
        position.insert(k.into(), v);
    }
    let end = match ligne_fin { Some(f) if f != ligne => Some(at(f)?), _ => None };
    // Le commentaire s'affiche sur la dernière ligne de la plage.
    line_fields(end.as_ref().unwrap_or(&start), &mut position);
    if let Some(end) = end {
        let point = |l: &DiffLine| {
            let mut p = serde_json::Map::new();
            p.insert("line_code".into(), json!(line_code(&new_path, l)));
            p.insert("type".into(), json!(if l.kind == LineType::New { "new" } else { "old" }));
            line_fields(l, &mut p);
            Value::Object(p)
        };
        position.insert("line_range".into(), json!({ "start": point(&start), "end": point(&end) }));
    }
    Ok(Value::Object(position))
}

// --- Mise en forme ---------------------------------------------------------------------------

fn user(u: &Value) -> Value { if u.is_object() { u["username"].clone() } else { Value::Null } }

fn or_undef(v: &Value) -> Value {
    match v { Value::String(s) if s.is_empty() => Value::Null, Value::Bool(false) => Value::Null, other => other.clone() }
}

fn project(p: &Value) -> Value {
    obj! {
        "id" => p["id"], "chemin" => p["path_with_namespace"], "nom" => p["name"], "description" => or_undef(&p["description"]),
        "brancheParDefaut" => p["default_branch"], "url" => p["web_url"], "derniereActivite" => p["last_activity_at"],
    }
}

fn mr(m: &Value) -> Value {
    obj! {
        "iid" => m["iid"], "titre" => m["title"], "etat" => m["state"],
        "brouillon" => if m["draft"].is_null() { m["work_in_progress"].clone() } else { m["draft"].clone() },
        "auteur" => user(&m["author"]), "source" => m["source_branch"], "cible" => m["target_branch"],
        "relecteurs" => m["reviewers"].as_array().map(|a| a.iter().map(user).collect::<Vec<_>>()).unwrap_or_default(),
        "libelles" => m["labels"],
        "fusion" => if m["merge_status"].is_null() { m["detailed_merge_status"].clone() } else { m["merge_status"].clone() },
        "maj" => m["updated_at"], "url" => m["web_url"],
    }
}

fn issue(i: &Value) -> Value {
    obj! {
        "iid" => i["iid"], "titre" => i["title"], "etat" => i["state"], "auteur" => user(&i["author"]),
        "assignes" => i["assignees"].as_array().map(|a| a.iter().map(user).collect::<Vec<_>>()).unwrap_or_default(),
        "libelles" => i["labels"], "jalon" => i["milestone"]["title"], "maj" => i["updated_at"], "url" => i["web_url"],
    }
}

fn pipeline(p: &Value) -> Value {
    obj! {
        "id" => p["id"], "statut" => p["status"], "ref" => p["ref"], "sha" => p["sha"], "source" => p["source"],
        "cree" => p["created_at"], "duree" => p["duration"], "url" => p["web_url"],
    }
}

fn job(j: &Value) -> Value {
    obj! {
        "id" => j["id"], "nom" => j["name"], "etape" => j["stage"], "statut" => j["status"], "echec" => j["failure_reason"],
        "duree" => j["duration"], "autoriseEchec" => or_undef(&j["allow_failure"]), "url" => j["web_url"],
    }
}

fn commit(c: &Value) -> Value {
    obj! {
        "sha" => c["id"], "titre" => c["title"], "auteur" => c["author_name"],
        "date" => if c["committed_date"].is_null() { c["created_at"].clone() } else { c["committed_date"].clone() },
        "url" => c["web_url"],
    }
}

fn note(n: &Value) -> Value {
    let p = &n["position"];
    obj! {
        "id" => n["id"], "auteur" => user(&n["author"]), "texte" => n["body"], "date" => n["created_at"],
        "systeme" => or_undef(&n["system"]),
        "resolu" => if n["resolvable"].as_bool() == Some(true) { n["resolved"].clone() } else { Value::Null },
        "position" => if p.is_object() {
            obj! {
                "fichier" => if p["new_path"].is_null() { p["old_path"].clone() } else { p["new_path"].clone() },
                "ligneNouvelle" => p["new_line"], "ligneAncienne" => p["old_line"],
            }
        } else { Value::Null },
    }
}

fn arr(v: &Value) -> &[Value] { v.as_array().map(|a| a.as_slice()).unwrap_or_default() }

// --- Paramètres communs ---------------------------------------------------------------------

fn page_fields() -> Vec<(&'static str, S)> {
    vec![
        ("page", z::int().min(1).optional().describe("Page (1 par défaut)")),
        ("parPage", z::int().min(1).max(100).optional().describe("Résultats par page (20 par défaut, 100 au plus)")),
    ]
}

fn with_pages(mut fields: Vec<(&'static str, S)>) -> Vec<(&'static str, S)> {
    fields.extend(page_fields());
    fields
}

fn project_s() -> S { z::string().min(1).describe("Projet : chemin « groupe/projet », id numérique, ou URL web GitLab") }

const MARKDOWN: &str = "Markdown GitLab.";

fn pages(a: &Value) -> Query { vec![("page", json!(a.int("page").unwrap_or(1))), ("per_page", json!(a.int("parPage").unwrap_or(20)))] }

fn q(mut base: Query, extra: Query) -> Query {
    base.extend(extra);
    base
}

fn enc(s: &str) -> String { crate::rancher::enc(s) }

/// Projet encodé pour l'API, et référence tirée de l'URL éventuelle.
fn reference(http: &ServiceHttp, input: &str) -> anyhow::Result<(GitlabRef, String)> {
    let r = parse_ref(&http.base_url(), input);
    if r.project.is_empty() { anyhow::bail!("Projet GitLab illisible : {input}"); }
    let api = format!("api/v4/projects/{}", enc(&r.project));
    Ok((r, api))
}

/// Numéro (MR, issue, pipeline, job) : paramètre, ou tiré de l'URL.
fn number_of(r: &GitlabRef, given: Option<i64>, kinds: &[&str], what: &str) -> anyhow::Result<i64> {
    given
        .or_else(|| r.kind.as_deref().filter(|k| kinds.contains(k)).and(r.id))
        .ok_or_else(|| anyhow::anyhow!("{what} manquant : donnez-le, ou l'URL web complète."))
}

fn mr_of(r: &GitlabRef, n: Option<i64>) -> anyhow::Result<i64> { number_of(r, n, &["merge_requests"], "Numéro de MR") }
fn issue_of(r: &GitlabRef, n: Option<i64>) -> anyhow::Result<i64> { number_of(r, n, &["issues"], "Numéro d'issue") }

async fn default_branch(http: &ServiceHttp, api: &str) -> anyhow::Result<String> {
    Ok(s(&http.get(api, vec![]).await?["default_branch"]))
}

fn opt(a: &Value, k: &str) -> Value { a.get(k).cloned().unwrap_or(Value::Null) }

fn note_url(base: &Value, n: &Value) -> String { format!("{}#note_{}", s(base), s(&n["id"])) }

pub fn tools() -> Vec<Tool> {
    const G: &str = "gitlab";
    let mut t = vec![];

    t.push(read(G, "gitlab_projets_chercher", "Chercher des projets",
        "Projets GitLab visibles par l'utilisateur, par nom ou chemin ; option : dans un groupe.",
        with_pages(vec![
            ("recherche", z::string().optional().describe("Texte cherché dans le nom ou le chemin")),
            ("groupe", z::string().optional().describe("Chemin du groupe (sous-groupes inclus)")),
            ("membre", z::boolean().optional().describe("Seulement les projets dont l'utilisateur est membre")),
        ]),
        |a, http| async move {
            let query = q(vec![("search", opt(&a, "recherche")), ("membership", opt(&a, "membre")), ("order_by", json!("last_activity_at")), ("simple", json!(true))], pages(&a));
            let list = match a.str("groupe") {
                Some(g) => http.get(&format!("api/v4/groups/{}/projects", enc(&g)), q(query, vec![("include_subgroups", json!(true))])).await?,
                None => http.get("api/v4/projects", query).await?,
            };
            Ok(Value::Array(arr(&list).iter().map(project).collect()))
        }));

    t.push(read(G, "gitlab_projet", "Détail d’un projet", "Projet GitLab : branche par défaut, URL, description, visibilité.",
        vec![("projet", project_s())],
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let p = http.get(&api, vec![]).await?;
            let mut out = project(&p);
            if let Some(o) = out.as_object_mut() {
                if !p["visibility"].is_null() { o.insert("visibilite".into(), p["visibility"].clone()); }
                if !p["open_issues_count"].is_null() { o.insert("ouvertes".into(), p["open_issues_count"].clone()); }
            }
            Ok(out)
        }));

    t.push(read(G, "gitlab_arborescence", "Arborescence",
        "Fichiers et dossiers d'un chemin du dépôt, à une référence (branche, tag, commit).",
        with_pages(vec![
            ("projet", project_s()),
            ("chemin", z::string().optional().describe("Dossier (racine par défaut)")),
            ("ref", z::string().optional().describe("Branche, tag ou commit (branche par défaut)")),
            ("recursif", z::boolean().optional()),
        ]),
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let list = http.get(&format!("{api}/repository/tree"), q(vec![("path", opt(&a, "chemin")), ("ref", opt(&a, "ref")), ("recursive", opt(&a, "recursif"))], pages(&a))).await?;
            Ok(Value::Array(arr(&list).iter().map(|e| json!({ "chemin": e["path"], "type": if e["type"] == json!("tree") { "dossier" } else { "fichier" } })).collect()))
        }));

    t.push(read(G, "gitlab_fichier_lire", "Lire un fichier",
        "Contenu brut d'un fichier du dépôt à une référence (branche par défaut sinon).",
        vec![
            ("projet", project_s()),
            ("chemin", z::string().min(1).describe("Chemin du fichier dans le dépôt")),
            ("ref", z::string().optional().describe("Branche, tag ou commit")),
        ],
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let at = match a.str("ref") { Some(r) => r, None => default_branch(&http, &api).await? };
            let path = enc(a.string("chemin").trim_start_matches('/'));
            Ok(Value::String(http.get_text(&format!("{api}/repository/files/{path}/raw"), vec![("ref", json!(at))]).await?))
        }));

    t.push(read(G, "gitlab_code_chercher", "Chercher dans le code",
        "Recherche de code (fichiers) dans un projet ou un groupe. Résultats : fichier, ligne de début, extrait.",
        with_pages(vec![
            ("recherche", z::string().min(1)),
            ("projet", project_s().optional()),
            ("groupe", z::string().optional().describe("Chemin du groupe (si pas de projet)")),
            ("ref", z::string().optional().describe("Branche ou tag (projet seulement)")),
        ]),
        |a, http| async move {
            let base = match (a.str("projet"), a.str("groupe")) {
                (Some(p), _) => reference(&http, &p)?.1,
                (None, Some(g)) => format!("api/v4/groups/{}", enc(&g)),
                _ => "api/v4".into(),
            };
            let reference = if a.str("projet").is_some() { opt(&a, "ref") } else { Value::Null };
            let list = http.get(&format!("{base}/search"), q(vec![("scope", json!("blobs")), ("search", opt(&a, "recherche")), ("ref", reference)], pages(&a))).await?;
            Ok(Value::Array(arr(&list).iter().map(|b| obj! {
                "projetId" => b["project_id"], "fichier" => if b["path"].is_null() { b["filename"].clone() } else { b["path"].clone() },
                "ref" => b["ref"], "ligne" => b["startline"], "extrait" => b["data"],
            }).collect()))
        }));

    t.push(read(G, "gitlab_commits", "Historique des commits", "Commits d’une branche, filtrables par chemin, auteur et dates.",
        with_pages(vec![
            ("projet", project_s()),
            ("ref", z::string().optional().describe("Branche ou tag (branche par défaut)")),
            ("chemin", z::string().optional()),
            ("auteur", z::string().optional()),
            ("depuis", z::string().optional().describe("Date ISO 8601")),
            ("jusqua", z::string().optional().describe("Date ISO 8601")),
        ]),
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let list = http.get(&format!("{api}/repository/commits"), q(vec![
                ("ref_name", opt(&a, "ref")), ("path", opt(&a, "chemin")), ("author", opt(&a, "auteur")),
                ("since", opt(&a, "depuis")), ("until", opt(&a, "jusqua")),
            ], pages(&a))).await?;
            Ok(Value::Array(arr(&list).iter().map(commit).collect()))
        }));

    t.push(read(G, "gitlab_commit_lire", "Détail d’un commit", "Commit : message complet, statistiques et diff des fichiers.",
        with_pages(vec![("projet", project_s()), ("sha", z::string().min(6))]),
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let sha = enc(&a.string("sha"));
            let c = http.get(&format!("{api}/repository/commits/{sha}"), vec![]).await?;
            let diffs = http.get(&format!("{api}/repository/commits/{sha}/diff"), pages(&a)).await?;
            let mut out = commit(&c);
            let o = out.as_object_mut().unwrap();
            for (k, v) in [("message", &c["message"]), ("stats", &c["stats"]), ("parents", &c["parent_ids"])] {
                if !v.is_null() { o.insert(k.into(), v.clone()); }
            }
            o.insert("fichiers".into(), Value::Array(arr(&diffs).iter().map(|d| obj! { "ancien" => d["old_path"], "nouveau" => d["new_path"], "diff" => d["diff"] }).collect()));
            Ok(out)
        }));

    t.push(read(G, "gitlab_branches", "Branches et tags", "Branches (ou tags) du dépôt, avec le dernier commit.",
        with_pages(vec![("projet", project_s()), ("type", z::enumeration(&["branches", "tags"]).optional()), ("recherche", z::string().optional())]),
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let kind = a.str("type").unwrap_or_else(|| "branches".into());
            let list = http.get(&format!("{api}/repository/{kind}"), q(vec![("search", opt(&a, "recherche"))], pages(&a))).await?;
            Ok(Value::Array(arr(&list).iter().map(|b| obj! {
                "nom" => b["name"], "protegee" => b["protected"], "parDefaut" => b["default"],
                "commit" => if b["commit"].is_object() { commit(&b["commit"]) } else { Value::Null },
            }).collect()))
        }));

    t.push(read(G, "gitlab_mr_lister", "Lister les merge requests",
        "MR d'un projet, ou de toute l'instance sans projet (portée : créées par moi, assignées à moi, toutes).",
        with_pages(vec![
            ("projet", project_s().optional()),
            ("etat", z::enumeration(&["opened", "closed", "merged", "all"]).optional().describe("opened par défaut")),
            ("portee", z::enumeration(&["created_by_me", "assigned_to_me", "all"]).optional()),
            ("auteur", z::string().optional().describe("Nom d'utilisateur")),
            ("relecteur", z::string().optional().describe("Nom d'utilisateur")),
            ("libelles", z::string().optional().describe("Libellés séparés par des virgules")),
            ("recherche", z::string().optional()),
        ]),
        |a, http| async move {
            let path = match a.str("projet") { Some(p) => format!("{}/merge_requests", reference(&http, &p)?.1), None => "api/v4/merge_requests".into() };
            let scope = a.get("portee").cloned().unwrap_or(if a.str("projet").is_some() { Value::Null } else { json!("all") });
            let list = http.get(&path, q(vec![
                ("state", json!(a.str("etat").unwrap_or_else(|| "opened".into()))), ("scope", scope),
                ("author_username", opt(&a, "auteur")), ("reviewer_username", opt(&a, "relecteur")),
                ("labels", opt(&a, "libelles")), ("search", opt(&a, "recherche")),
            ], pages(&a))).await?;
            Ok(Value::Array(arr(&list).iter().map(mr).collect()))
        }));

    t.push(read(G, "gitlab_mr_lire", "Lire une merge request",
        "MR : description, statut, pipeline, approbations, références du diff. Suite : gitlab_mr_diff, gitlab_mr_discussions.",
        vec![("projet", project_s()), ("mr", z::int().min(1).optional().describe("Numéro (iid) ; inutile avec l'URL"))],
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let iid = mr_of(&r, a.int("mr"))?;
            let m = http.get(&format!("{api}/merge_requests/{iid}"), vec![]).await?;
            // Approbations indisponibles (édition, droits) : ignorées.
            let approvals = http.get(&format!("{api}/merge_requests/{iid}/approvals"), vec![]).await.ok().map(|ap| obj! {
                "approuvee" => ap["approved"], "restantes" => ap["approvals_left"],
                "par" => arr(&ap["approved_by"]).iter().map(|x| user(&x["user"])).collect::<Vec<_>>(),
            });
            let mut out = mr(&m);
            let o = out.as_object_mut().unwrap();
            let extra = obj! {
                "description" => m["description"],
                "assignes" => arr(&m["assignees"]).iter().map(user).collect::<Vec<_>>(),
                "conflits" => m["has_conflicts"],
                "fils" => if m["blocking_discussions_resolved"] == json!(false) { json!("discussions bloquantes non résolues") } else { Value::Null },
                "pipeline" => if m["head_pipeline"].is_object() { pipeline(&m["head_pipeline"]) } else { Value::Null },
                "diffRefs" => m["diff_refs"],
                "approbations" => approvals,
            };
            o.extend(extra.as_object().unwrap().clone());
            Ok(out)
        }));

    t.push(read(G, "gitlab_mr_diff", "Diff d’une merge request",
        "Fichiers modifiés par la MR avec leur diff unifié (paginé). Les numéros de ligne servent aux commentaires positionnés.",
        with_pages(vec![("projet", project_s()), ("mr", z::int().min(1).optional()), ("fichier", z::string().optional().describe("Seulement ce fichier (chemin)"))]),
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let iid = mr_of(&r, a.int("mr"))?;
            let files = match http.get(&format!("{api}/merge_requests/{iid}/diffs"), pages(&a)).await {
                Ok(f) => f,
                // GitLab < 15.7 : ancien point d'accès, non paginé.
                Err(e) if e.status == Some(404) => http.get(&format!("{api}/merge_requests/{iid}/changes"), vec![]).await?["changes"].clone(),
                Err(e) => return Err(e.into()),
            };
            let only = a.str("fichier");
            Ok(Value::Array(arr(&files).iter()
                .filter(|f| only.as_ref().is_none_or(|o| f["new_path"] == json!(o) || f["old_path"] == json!(o)))
                .map(|f| obj! {
                    "ancien" => f["old_path"], "nouveau" => f["new_path"], "cree" => or_undef(&f["new_file"]),
                    "supprime" => or_undef(&f["deleted_file"]), "renomme" => or_undef(&f["renamed_file"]), "diff" => f["diff"],
                }).collect()))
        }));

    t.push(read(G, "gitlab_mr_discussions", "Discussions d’une merge request",
        "Fils de discussion de la MR (commentaires, position fichier/ligne, résolution). Id du fil pour y répondre.",
        with_pages(vec![("projet", project_s()), ("mr", z::int().min(1).optional()), ("systeme", z::boolean().optional().describe("Inclure les notes système"))]),
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let list = http.get(&format!("{api}/merge_requests/{}/discussions", mr_of(&r, a.int("mr"))?), pages(&a)).await?;
            let system = a.bool("systeme").unwrap_or(false);
            Ok(Value::Array(arr(&list).iter().filter_map(|d| {
                let notes: Vec<Value> = arr(&d["notes"]).iter().filter(|n| system || n["system"] != json!(true)).map(note).collect();
                (!notes.is_empty()).then(|| json!({ "fil": d["id"], "notes": notes }))
            }).collect()))
        }));

    t.push(read(G, "gitlab_issues_lister", "Lister les issues", "Issues d'un projet, ou de toute l'instance sans projet.",
        with_pages(vec![
            ("projet", project_s().optional()),
            ("etat", z::enumeration(&["opened", "closed", "all"]).optional()),
            ("portee", z::enumeration(&["created_by_me", "assigned_to_me", "all"]).optional()),
            ("libelles", z::string().optional()),
            ("recherche", z::string().optional()),
            ("assigne", z::string().optional().describe("Nom d'utilisateur")),
        ]),
        |a, http| async move {
            let path = match a.str("projet") { Some(p) => format!("{}/issues", reference(&http, &p)?.1), None => "api/v4/issues".into() };
            let scope = a.get("portee").cloned().unwrap_or(if a.str("projet").is_some() { Value::Null } else { json!("all") });
            let list = http.get(&path, q(vec![
                ("state", json!(a.str("etat").unwrap_or_else(|| "opened".into()))), ("scope", scope), ("labels", opt(&a, "libelles")),
                ("search", opt(&a, "recherche")), ("assignee_username", opt(&a, "assigne")),
            ], pages(&a))).await?;
            Ok(Value::Array(arr(&list).iter().map(issue).collect()))
        }));

    t.push(read(G, "gitlab_issue_lire", "Lire une issue", "Issue avec sa description et ses commentaires.",
        with_pages(vec![("projet", project_s()), ("issue", z::int().min(1).optional())]),
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let iid = issue_of(&r, a.int("issue"))?;
            let i = http.get(&format!("{api}/issues/{iid}"), vec![]).await?;
            let notes = http.get(&format!("{api}/issues/{iid}/notes"), q(vec![("sort", json!("asc")), ("order_by", json!("created_at"))], pages(&a))).await?;
            let mut out = issue(&i);
            let o = out.as_object_mut().unwrap();
            if !i["description"].is_null() { o.insert("description".into(), i["description"].clone()); }
            o.insert("commentaires".into(), Value::Array(arr(&notes).iter().filter(|n| n["system"] != json!(true)).map(note).collect()));
            Ok(out)
        }));

    t.push(read(G, "gitlab_pipelines_lister", "Lister les pipelines",
        "Pipelines du projet (les plus récents d’abord), filtrables par branche et statut.",
        with_pages(vec![
            ("projet", project_s()),
            ("ref", z::string().optional()),
            ("statut", z::enumeration(&["running", "pending", "success", "failed", "canceled", "skipped", "manual", "created"]).optional()),
        ]),
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let list = http.get(&format!("{api}/pipelines"), q(vec![("ref", opt(&a, "ref")), ("status", opt(&a, "statut"))], pages(&a))).await?;
            Ok(Value::Array(arr(&list).iter().map(pipeline).collect()))
        }));

    t.push(read(G, "gitlab_pipeline_lire", "Lire un pipeline", "Pipeline et ses jobs (étape, statut, raison d’échec). Logs : gitlab_job_log.",
        vec![("projet", project_s()), ("pipeline", z::int().min(1).optional())],
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let id = number_of(&r, a.int("pipeline"), &["pipelines"], "Numéro de pipeline")?;
            let p = http.get(&format!("{api}/pipelines/{id}"), vec![]).await?;
            let jobs = http.get(&format!("{api}/pipelines/{id}/jobs"), vec![("per_page", json!(100)), ("include_retried", json!(false))]).await?;
            let mut out = pipeline(&p);
            out.as_object_mut().unwrap().insert("jobs".into(), Value::Array(arr(&jobs).iter().map(job).collect()));
            Ok(out)
        }));

    t.push(read(G, "gitlab_job_log", "Log d’un job", "Log (trace) d’un job CI, sans codes de couleur. « fin » : seulement les N dernières lignes.",
        vec![("projet", project_s()), ("job", z::int().min(1).optional()), ("fin", z::int().min(1).optional().describe("N dernières lignes"))],
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let id = number_of(&r, a.int("job"), &["jobs"], "Numéro de job")?;
            let text = clean_job_log(&http.get_text(&format!("{api}/jobs/{id}/trace"), vec![]).await?);
            Ok(Value::String(match a.int("fin") {
                None => text,
                Some(n) => {
                    let lines: Vec<&str> = text.split('\n').collect();
                    lines[lines.len().saturating_sub(n as usize)..].join("\n")
                }
            }))
        }));

    // --- Écriture -------------------------------------------------------------------------

    let mr_target = |a: &Value| format!("{}{}", a.string("projet"), a.int("mr").map(|n| format!(" !{n}")).unwrap_or_default());

    t.push(write(G, "gitlab_mr_commenter", "Commenter une merge request",
        &format!("Commentaire global, ou positionné sur une ligne ou une plage du diff (paramètre position). Ligne « nouveau » : ligne ajoutée ou inchangée, numérotée dans la nouvelle version ; « ancien » : ligne supprimée, numérotée dans l’ancienne. {MARKDOWN}"),
        vec![
            ("projet", project_s()),
            ("mr", z::int().min(1).optional()),
            ("texte", z::string().min(1)),
            ("position", z::object(vec![
                ("fichier", z::string().min(1).describe("Chemin du fichier (nouveau chemin si renommé)")),
                ("ligne", z::int().min(1)),
                ("cote", z::enumeration(&["nouveau", "ancien"]).optional().describe("nouveau par défaut")),
                ("ligneFin", z::int().min(1).optional().describe("Dernière ligne, pour une plage")),
            ]).optional().describe("Absent : commentaire global")),
        ],
        move |a| Ok(format!("{}{}", mr_target(a), if a["position"].is_object() { format!(" {}:{}", s(&a["position"]["fichier"]), s(&a["position"]["ligne"])) } else { String::new() })),
        false,
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let iid = mr_of(&r, a.int("mr"))?;
            let m = http.get(&format!("{api}/merge_requests/{iid}"), vec![]).await?;
            let body = sign(&a.string("texte"), SignatureFormat::Markdown);
            let pos = &a["position"];
            if !pos.is_object() {
                let n = http.post(&format!("{api}/merge_requests/{iid}/notes"), json!({ "body": body })).await?;
                let url = note_url(&m["web_url"], &n);
                return Ok(WriteResult { value: json!({ "note": n["id"], "url": url }), url: Some(url) });
            }
            let versions = http.get(&format!("{api}/merge_requests/{iid}/versions"), vec![]).await?;
            let Some(first) = arr(&versions).first() else { anyhow::bail!("MR sans diff : faites un commentaire global.") };
            let version = http.get(&format!("{api}/merge_requests/{iid}/versions/{}", s(&first["id"])), vec![]).await?;
            let fichier = s(&pos["fichier"]);
            let file = arr(&version["diffs"]).iter().find(|d| d["new_path"] == json!(fichier) || d["old_path"] == json!(fichier)).cloned();
            let Some(file) = file else {
                let names: Vec<String> = arr(&version["diffs"]).iter().take(50).map(|d| s(&d["new_path"])).collect();
                anyhow::bail!("{fichier} n'est pas modifié par la MR !{iid}. Fichiers : {}.", names.join(", "));
            };
            let position = build_position(first, &file, pos["ligne"].as_i64().unwrap_or(0), pos["cote"] == json!("ancien"), pos["ligneFin"].as_i64())?;
            let d = http.post(&format!("{api}/merge_requests/{iid}/discussions"), json!({ "body": body, "position": position })).await?;
            let first_note = arr(&d["notes"]).first().cloned().unwrap_or_default();
            let url = note_url(&m["web_url"], &first_note);
            let positioned = first_note["position"].is_object();
            let mut value = json!({ "fil": d["id"], "note": first_note["id"], "url": url, "positionne": positioned });
            if !positioned { value["avertissement"] = json!("GitLab a enregistré le commentaire sans position (commentaire global)."); }
            Ok(WriteResult { value, url: Some(url) })
        }));

    t.push(write(G, "gitlab_discussion_repondre", "Répondre dans un fil",
        &format!("Réponse dans un fil de discussion de MR (id du fil : gitlab_mr_discussions). {MARKDOWN}"),
        vec![("projet", project_s()), ("mr", z::int().min(1).optional()), ("fil", z::string().min(1)), ("texte", z::string().min(1))],
        move |a| Ok(format!("{} fil {}", mr_target(a), a.string("fil").chars().take(8).collect::<String>())),
        false,
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let iid = mr_of(&r, a.int("mr"))?;
            let m = http.get(&format!("{api}/merge_requests/{iid}"), vec![]).await?;
            let n = http.post(&format!("{api}/merge_requests/{iid}/discussions/{}/notes", enc(&a.string("fil"))), json!({ "body": sign(&a.string("texte"), SignatureFormat::Markdown) })).await?;
            let url = note_url(&m["web_url"], &n);
            Ok(WriteResult { value: json!({ "note": n["id"], "url": url }), url: Some(url) })
        }));

    t.push(write(G, "gitlab_discussion_resoudre", "Résoudre un fil", "Marque un fil de discussion de MR comme résolu (ou le rouvre).",
        vec![("projet", project_s()), ("mr", z::int().min(1).optional()), ("fil", z::string().min(1)), ("resolu", z::boolean().optional().describe("true par défaut ; false pour rouvrir"))],
        move |a| Ok(format!("{} fil {}", mr_target(a), a.string("fil").chars().take(8).collect::<String>())),
        false,
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let iid = mr_of(&r, a.int("mr"))?;
            let resolved = a.bool("resolu").unwrap_or(true);
            let d = http.put(&format!("{api}/merge_requests/{iid}/discussions/{}", enc(&a.string("fil"))), None, vec![("resolved", json!(resolved))]).await?;
            let m = http.get(&format!("{api}/merge_requests/{iid}"), vec![]).await?;
            Ok(WriteResult { value: json!({ "fil": d["id"], "resolu": resolved }), url: m["web_url"].as_str().map(str::to_string) })
        }));

    t.push(write(G, "gitlab_mr_creer", "Créer une merge request", &format!("Crée une MR (cible : branche par défaut si absente). {MARKDOWN}"),
        vec![
            ("projet", project_s()),
            ("source", z::string().min(1).describe("Branche source (doit exister)")),
            ("cible", z::string().optional()),
            ("titre", z::string().min(1)),
            ("description", z::string().optional()),
            ("brouillon", z::boolean().optional()),
            ("libelles", z::string().optional().describe("Séparés par des virgules")),
            ("supprimerSource", z::boolean().optional().describe("Supprimer la branche source après fusion")),
        ],
        |a| Ok(format!("{} {} → {}", a.string("projet"), a.string("source"), a.str("cible").unwrap_or_else(|| "(défaut)".into()))),
        false,
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let target = match a.str("cible") { Some(c) => c, None => default_branch(&http, &api).await? };
            let title = a.string("titre");
            let title = if a.bool("brouillon") == Some(true) && !title.to_lowercase().starts_with("draft:") { format!("Draft: {title}") } else { title };
            let m = http.post(&format!("{api}/merge_requests"), obj! {
                "source_branch" => a["source"], "target_branch" => target, "title" => title,
                "description" => sign(&a.string("description"), SignatureFormat::Markdown),
                "labels" => opt(&a, "libelles"), "remove_source_branch" => opt(&a, "supprimerSource"),
            }).await?;
            Ok(WriteResult { value: mr(&m), url: m["web_url"].as_str().map(str::to_string) })
        }));

    t.push(write(G, "gitlab_issue_creer", "Créer une issue", &format!("Crée une issue dans le projet. {MARKDOWN}"),
        vec![
            ("projet", project_s()),
            ("titre", z::string().min(1)),
            ("description", z::string().optional()),
            ("libelles", z::string().optional()),
            ("assignes", z::array(z::string()).optional().describe("Noms d'utilisateur")),
        ],
        |a| Ok(format!("{} : {}", a.string("projet"), a.string("titre").chars().take(80).collect::<String>())),
        false,
        |a, http| async move {
            let (_, api) = reference(&http, &a.string("projet"))?;
            let mut assignees: Option<Vec<Value>> = None;
            if let Some(names) = a["assignes"].as_array().filter(|n| !n.is_empty()) {
                let mut ids = vec![];
                for username in names {
                    let found = http.get("api/v4/users", vec![("username", username.clone())]).await?;
                    let Some(u) = arr(&found).first() else { anyhow::bail!("Utilisateur GitLab inconnu : {}", s(username)) };
                    ids.push(u["id"].clone());
                }
                assignees = Some(ids);
            }
            let i = http.post(&format!("{api}/issues"), obj! {
                "title" => a["titre"], "description" => sign(&a.string("description"), SignatureFormat::Markdown),
                "labels" => opt(&a, "libelles"), "assignee_ids" => assignees,
            }).await?;
            Ok(WriteResult { value: issue(&i), url: i["web_url"].as_str().map(str::to_string) })
        }));

    t.push(write(G, "gitlab_issue_commenter", "Commenter une issue", &format!("Ajoute un commentaire à une issue. {MARKDOWN}"),
        vec![("projet", project_s()), ("issue", z::int().min(1).optional()), ("texte", z::string().min(1))],
        |a| Ok(format!("{}{}", a.string("projet"), a.int("issue").map(|n| format!(" #{n}")).unwrap_or_default())),
        false,
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let iid = issue_of(&r, a.int("issue"))?;
            let i = http.get(&format!("{api}/issues/{iid}"), vec![]).await?;
            let n = http.post(&format!("{api}/issues/{iid}/notes"), json!({ "body": sign(&a.string("texte"), SignatureFormat::Markdown) })).await?;
            let url = note_url(&i["web_url"], &n);
            Ok(WriteResult { value: json!({ "note": n["id"], "url": url }), url: Some(url) })
        }));

    t.push(write(G, "gitlab_pipeline_relancer", "Relancer un pipeline", "Relance les jobs en échec ou annulés d’un pipeline.",
        vec![("projet", project_s()), ("pipeline", z::int().min(1).optional())],
        |a| Ok(format!("{}{}", a.string("projet"), a.int("pipeline").map(|n| format!(" pipeline {n}")).unwrap_or_default())),
        false,
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let id = number_of(&r, a.int("pipeline"), &["pipelines"], "Numéro de pipeline")?;
            let p = http.post_empty(&format!("{api}/pipelines/{id}/retry")).await?;
            Ok(WriteResult { value: pipeline(&p), url: p["web_url"].as_str().map(str::to_string) })
        }));

    t.push(write(G, "gitlab_job_relancer", "Relancer un job", "Relance un job CI (crée un nouveau job).",
        vec![("projet", project_s()), ("job", z::int().min(1).optional())],
        |a| Ok(format!("{}{}", a.string("projet"), a.int("job").map(|n| format!(" job {n}")).unwrap_or_default())),
        false,
        |a, http| async move {
            let (r, api) = reference(&http, &a.string("projet"))?;
            let id = number_of(&r, a.int("job"), &["jobs"], "Numéro de job")?;
            let j = http.post_empty(&format!("{api}/jobs/{id}/retry")).await?;
            Ok(WriteResult { value: job(&j), url: j["web_url"].as_str().map(str::to_string) })
        }));

    t
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const FAKE_DIFF: &str = "@@ -1,4 +1,5 @@\n un\n-deux\n+deux bis\n+nouvelle\n trois\n quatre\n";

    #[test]
    fn numerotation_comme_gitlab() {
        let lines = parse_diff(FAKE_DIFF);
        let l = |kind, old_line, new_line| DiffLine { kind, old_line, new_line };
        assert_eq!(lines, vec![
            l(LineType::Context, 1, 1), l(LineType::Old, 2, 2), l(LineType::New, 3, 2),
            l(LineType::New, 3, 3), l(LineType::Context, 3, 4), l(LineType::Context, 4, 5),
        ]);
        assert_eq!(find_line(&lines, 2, false).unwrap().kind, LineType::Old);
        assert_eq!(find_line(&lines, 3, true).unwrap().kind, LineType::New);
        assert_eq!(find_line(&lines, 4, true).unwrap().kind, LineType::Context);
    }

    #[test]
    fn positions() {
        let refs = json!({ "base_commit_sha": "b", "start_commit_sha": "s", "head_commit_sha": "h" });
        let file = json!({ "old_path": "src/a.ts", "new_path": "src/a.ts", "diff": FAKE_DIFF });
        let p = build_position(&refs, &file, 3, false, None).unwrap();
        assert_eq!(p, json!({ "position_type": "text", "base_sha": "b", "start_sha": "s", "head_sha": "h", "old_path": "src/a.ts", "new_path": "src/a.ts", "new_line": 3 }));
        assert_eq!(build_position(&refs, &file, 2, true, None).unwrap()["old_line"], json!(2));
        let p = build_position(&refs, &file, 5, false, None).unwrap();
        assert_eq!((p["old_line"].clone(), p["new_line"].clone()), (json!(4), json!(5)));
        let range = build_position(&refs, &file, 2, false, Some(4)).unwrap();
        let start = find_line(&parse_diff(FAKE_DIFF), 2, true).unwrap();
        assert_eq!((range["new_line"].clone(), range["old_line"].clone()), (json!(4), json!(3)));
        assert_eq!(range["line_range"]["start"], json!({ "line_code": line_code("src/a.ts", &start), "type": "new", "new_line": 2 }));
        assert_eq!(range["line_range"]["end"], json!({ "line_code": line_code("src/a.ts", &find_line(&parse_diff(FAKE_DIFF), 4, true).unwrap()), "type": "old", "old_line": 3, "new_line": 4 }));
        let e = build_position(&refs, &file, 40, false, None).unwrap_err().to_string();
        assert!(e.contains("Ligne 40 (nouvelle version) hors des modifications de src/a.ts. Lignes commentables : 1-5"), "{e}");
    }

    #[test]
    fn references() {
        assert_eq!(parse_ref("https://git.intra", "grp/app"), GitlabRef { project: "grp/app".into(), ..Default::default() });
        assert_eq!(
            parse_ref("https://git.intra/gitlab", "https://git.intra/gitlab/grp/sous/app/-/merge_requests/12#note_3"),
            GitlabRef { project: "grp/sous/app".into(), kind: Some("merge_requests".into()), id: Some(12) }
        );
        assert_eq!(parse_ref("https://git.intra", "https://git.intra/grp/app/-/jobs/99"), GitlabRef { project: "grp/app".into(), kind: Some("jobs".into()), id: Some(99) });
        assert_eq!(clean_job_log("section_start:1:build\r\x1b[0K\x1b[32;1mOK\x1b[0;m\n"), "OK\n");
    }
}
