//! S3 : test, buckets, explorateur (dossiers, aperçu) et opérations longues (téléchargement,
//! envoi en plusieurs parties, renommage, suppression) avec progression et annulation.

pub mod client;
pub mod service;

use crate::config::S3Connection;
use crate::error::{ErrorCode, SourceError};
use crate::rancher::SourceNode;
use base64::Engine;
use bytes::Bytes;
use client::{S3Client, S3Error, to_source_error};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

/// Taille maximale lue pour un aperçu (début du fichier).
pub const PREVIEW_BYTES: usize = 5 * 1024 * 1024;
const PART_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct ListResult {
    is_truncated: bool,
    next_continuation_token: Option<String>,
    contents: Vec<Object>,
    common_prefixes: Vec<CommonPrefix>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Object {
    key: String,
    size: u64,
    last_modified: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct CommonPrefix {
    prefix: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct BucketsResult {
    buckets: Buckets,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Buckets {
    bucket: Vec<Bucket>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Bucket {
    name: String,
}

fn xml<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, S3Error> {
    quick_xml::de::from_str(text).map_err(|e| S3Error { status: Some(200), code: "UnknownError".into(), message: format!("XML illisible : {e}"), source: None })
}

/// Entrée d'un dossier S3 (`key` complète ; un dossier se termine par `/`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct S3Entry {
    pub key: String,
    pub name: String,
    /// `folder` ou `file`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

impl S3Entry {
    pub fn is_folder(&self) -> bool { self.kind == "folder" }
    /// Entrée à partir d'une clé (dossier si elle finit par `/`).
    pub fn from_key(key: &str) -> Self {
        S3Entry { key: key.into(), name: base_name(key).into(), kind: if key.ends_with('/') { "folder" } else { "file" }.into(), size: None, modified: None }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct S3Page {
    pub entries: Vec<S3Entry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct S3Preview {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gunzipped: Option<bool>,
}

pub fn base_name(key: &str) -> &str {
    key.trim_end_matches('/').rsplit('/').next().unwrap_or(key)
}

fn extension(key: &str) -> String { key.rsplit('.').next().unwrap_or_default().to_lowercase() }

fn image_type(key: &str) -> Option<&'static str> {
    Some(match extension(key).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

/// Buckets visibles par la clé ; `None` si la clé n'a pas le droit de les lister.
async fn listed(client: &S3Client) -> Result<Option<Vec<String>>, S3Error> {
    match client.send("GET", None, None, &[]).await {
        Ok(res) => {
            let r: BucketsResult = xml(&S3Client::text(res).await?)?;
            Ok(Some(r.buckets.bucket.into_iter().map(|b| b.name).filter(|n| !n.is_empty()).collect()))
        }
        Err(e) if to_source_error(&e, "").code == ErrorCode::Forbidden => Ok(None),
        Err(e) => Err(e),
    }
}

/// Test : liste des buckets, sinon accès au premier bucket saisi à la main.
pub async fn test(connection: &S3Connection, secret: &str) -> Result<String, SourceError> {
    let client = S3Client::new(connection, secret)?;
    let names = listed(&client).await.map_err(|e| to_source_error(&e, ""))?;
    if let Some(names) = names { return Ok(format!("{} bucket(s) visible(s)", names.len())); }
    let Some(first) = connection.buckets.first() else {
        return Err(SourceError::new(
            ErrorCode::Forbidden,
            "Connexion établie, mais cette clé ne peut pas lister les buckets : saisissez le nom d'un bucket.",
        ));
    };
    client.send("HEAD", Some(first), None, &[]).await.map_err(|e| to_source_error(&e, first))?;
    Ok(format!("Bucket {first} accessible (liste des buckets non autorisée)"))
}

/// Buckets listés + buckets saisis à la main, triés, sans doublon.
pub async fn list_buckets(connection: &S3Connection, secret: &str) -> Result<Vec<SourceNode>, SourceError> {
    let client = S3Client::new(connection, secret)?;
    let mut names = listed(&client).await.map_err(|e| to_source_error(&e, ""))?.unwrap_or_default();
    for b in &connection.buckets { if !names.contains(b) { names.push(b.clone()); } }
    names.sort_by(|a, b| crate::text::locale_cmp(a, b));
    names.dedup();
    Ok(names.into_iter().map(|id| SourceNode { id, kind: "bucket", expandable: false, ..Default::default() }).collect())
}

async fn list_objects(client: &S3Client, bucket: &str, prefix: &str, delimiter: bool, max: Option<u32>, token: Option<&str>) -> Result<ListResult, S3Error> {
    let mut q: Vec<(&str, String)> = vec![("list-type", "2".into()), ("prefix", prefix.into())];
    if delimiter { q.push(("delimiter", "/".into())); }
    if let Some(m) = max { q.push(("max-keys", m.to_string())); }
    if let Some(t) = token { q.push(("continuation-token", t.into())); }
    let res = client.send("GET", Some(bucket), None, &q).await?;
    xml(&S3Client::text(res).await?)
}

/// Un dossier S3 (délimiteur `/`), une page à la fois.
pub async fn list_folder(client: &S3Client, bucket: &str, prefix: &str, search: &str, continuation: Option<&str>) -> Result<S3Page, S3Error> {
    let out = list_objects(client, bucket, &format!("{prefix}{search}"), true, Some(1000), continuation).await?;
    let folders = out.common_prefixes.into_iter().filter(|p| !p.prefix.is_empty()).map(|p| S3Entry {
        name: base_name(&p.prefix).into(),
        key: p.prefix,
        kind: "folder".into(),
        size: None,
        modified: None,
    });
    // Le « marqueur » d'un dossier (clé = préfixe) n'est pas un fichier.
    let files = out.contents.into_iter().filter(|o| !o.key.is_empty() && o.key != prefix).map(|o| S3Entry {
        name: base_name(&o.key).into(),
        modified: crate::text::to_iso(&o.last_modified).or(Some(o.last_modified).filter(|m| !m.is_empty())),
        size: Some(o.size),
        key: o.key,
        kind: "file".into(),
    });
    Ok(S3Page { entries: folders.chain(files).collect(), next: if out.is_truncated { out.next_continuation_token } else { None } })
}

/// Toutes les clés sous un préfixe (récursif).
pub async fn all_keys(client: &S3Client, bucket: &str, prefix: &str, cancel: &CancellationToken) -> Result<Vec<(String, u64)>, S3Error> {
    let mut keys = vec![];
    let mut token: Option<String> = None;
    loop {
        if cancel.is_cancelled() { return Err(cancelled()); }
        let out = list_objects(client, bucket, prefix, false, None, token.as_deref()).await?;
        keys.extend(out.contents.into_iter().filter(|o| !o.key.is_empty()).map(|o| (o.key, o.size)));
        token = if out.is_truncated { out.next_continuation_token } else { None };
        if token.is_none() { break; }
    }
    Ok(keys)
}

pub async fn count_keys(client: &S3Client, bucket: &str, prefix: &str) -> Result<usize, S3Error> {
    Ok(all_keys(client, bucket, prefix, &CancellationToken::new()).await?.len())
}

fn cancelled() -> S3Error {
    S3Error { status: None, code: "Cancelled".into(), message: "Opération annulée".into(), source: None }
}

fn header(res: &reqwest::Response, name: &str) -> Option<String> {
    res.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// Aperçu : début du fichier, décompressé si gzip ; image en `data:` URL si assez petite.
pub async fn preview(client: &S3Client, bucket: &str, key: &str) -> Result<S3Preview, S3Error> {
    let head = client.send("HEAD", Some(bucket), Some(key), &[]).await?;
    let size: u64 = header(&head, "content-length").and_then(|s| s.parse().ok()).unwrap_or(0);
    let content_type = header(&head, "content-type");
    let image = image_type(key);
    if image.is_some() && size > PREVIEW_BYTES as u64 {
        return Ok(S3Preview { kind: "binary", size, content_type, truncated: true, ..Default::default() });
    }
    let mut bytes: Vec<u8> = if size > 0 {
        let length = size.min(PREVIEW_BYTES as u64);
        let res = client
            .request("GET", Some(bucket), Some(key), &[], &[("range".into(), format!("bytes=0-{}", length - 1))], None, Duration::from_secs(60))
            .await?;
        let mut data = res.bytes().await.map_err(|e| S3Error { status: None, code: String::new(), message: e.to_string(), source: None })?.to_vec();
        data.truncate(length as usize);
        data
    } else {
        vec![]
    };
    let truncated = size > bytes.len() as u64;
    if let Some(t) = image {
        return Ok(S3Preview {
            kind: "image",
            data_url: Some(format!("data:{t};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes))),
            size,
            content_type,
            truncated: false,
            ..Default::default()
        });
    }
    let mut gunzipped = false;
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        bytes = gunzip_head(&bytes, PREVIEW_BYTES);
        gunzipped = true;
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Ok(S3Preview { kind: "binary", size, content_type, truncated, gunzipped: Some(gunzipped), ..Default::default() });
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(S3Preview {
        kind: "text",
        truncated: truncated || (gunzipped && bytes.len() >= PREVIEW_BYTES),
        text: Some(text),
        size,
        content_type,
        gunzipped: Some(gunzipped),
        ..Default::default()
    })
}

/// Décompresse le début d'un gzip (éventuellement tronqué), au plus `limit` octets.
fn gunzip_head(input: &[u8], limit: usize) -> Vec<u8> {
    let mut out = vec![];
    let mut decoder = flate2::read::MultiGzDecoder::new(input);
    let mut buf = [0u8; 64 * 1024];
    while out.len() < limit {
        match decoder.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
    out.truncate(limit);
    out
}

// --- Opérations longues -------------------------------------------------------------------

/// Avancement : objets traités, total (si connu), octets, objet en cours.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
}

pub struct OpContext {
    pub client: Arc<S3Client>,
    pub bucket: String,
    pub cancel: CancellationToken,
    pub report: Arc<dyn Fn(Progress) + Send + Sync>,
}

/// Exécute `f` sur chaque élément, `limit` à la fois ; s'arrête à la première erreur.
async fn pool<T, F, Fut>(items: Vec<T>, limit: usize, cancel: &CancellationToken, f: F) -> Result<(), S3Error>
where
    T: Send,
    F: Fn(T) -> Fut,
    Fut: std::future::Future<Output = Result<(), S3Error>>,
{
    let mut stream = futures_util::stream::iter(items.into_iter().map(|i| {
        let fut = f(i);
        let cancel = cancel.clone();
        async move {
            tokio::select! {
                _ = cancel.cancelled() => Err(cancelled()),
                r = fut => r,
            }
        }
    }))
    .buffer_unordered(limit.max(1));
    while let Some(r) = stream.next().await { r?; }
    if cancel.is_cancelled() { return Err(cancelled()); }
    Ok(())
}

/// Clés visées : les fichiers, et tout le contenu des dossiers. `base` : préfixe retiré
/// pour obtenir le chemin relatif (le dossier lui-même est gardé).
async fn expand(ctx: &OpContext, entries: &[S3Entry]) -> Result<Vec<(String, u64, String)>, S3Error> {
    let mut keys = vec![];
    for e in entries {
        let base = parent_of(&e.key);
        if e.is_folder() {
            for (k, s) in all_keys(&ctx.client, &ctx.bucket, &e.key, &ctx.cancel).await? { keys.push((k, s, base.clone())); }
        } else {
            keys.push((e.key.clone(), e.size.unwrap_or(0), base));
        }
    }
    Ok(keys)
}

/// Dossier parent d'une clé (`a/b/c.txt` → `a/b/`, `a/b/` → `a/`).
pub fn parent_of(key: &str) -> String {
    let trimmed = key.strip_suffix('/').unwrap_or(key);
    match trimmed.rfind('/') { Some(i) => trimmed[..=i].to_string(), None => String::new() }
}

/// Nom de fichier Windows valide (caractères interdits remplacés, pas de `..`).
fn safe_segment(segment: &str) -> String {
    let cleaned: String = segment.chars().map(|c| if "<>:\"|?*\\".contains(c) || (c as u32) < 0x20 { '_' } else { c }).collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() || cleaned == ".." || cleaned == "." { "_".into() } else { cleaned }
}

pub fn local_path(dir: &Path, relative: &str) -> PathBuf {
    let mut p = dir.to_path_buf();
    for s in relative.split('/').filter(|s| !s.is_empty()) { p.push(safe_segment(s)); }
    p
}

fn io_err(e: std::io::Error) -> S3Error {
    S3Error { status: None, code: "IoError".into(), message: e.to_string(), source: Some(SourceError::invalid(e.to_string())) }
}

/// Télécharge un fichier vers `file`, ou des fichiers / dossiers dans `dir`.
pub async fn download(ctx: &OpContext, entries: &[S3Entry], file: Option<&Path>, dir: Option<&Path>) -> Result<(), S3Error> {
    let keys = expand(ctx, entries).await?;
    let total = keys.len();
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    (ctx.report)(Progress { done: 0, total: Some(total), bytes: Some(0), current: None });
    let single = file.filter(|_| entries.len() == 1 && !entries[0].is_folder());
    pool(keys, 4, &ctx.cancel, |(key, _, base)| {
        let (done, bytes) = (&done, &bytes);
        async move {
            let destination = match single {
                Some(f) => f.to_path_buf(),
                None => local_path(dir.unwrap(), &key[base.len()..]),
            };
            if key.ends_with('/') {
                tokio::fs::create_dir_all(&destination).await.map_err(io_err)?;
            } else {
                if let Some(parent) = destination.parent() { tokio::fs::create_dir_all(parent).await.map_err(io_err)?; }
                let res = ctx.client.request("GET", Some(&ctx.bucket), Some(&key), &[], &[], None, Duration::from_secs(3600)).await?;
                let mut out = tokio::fs::File::create(&destination).await.map_err(io_err)?;
                let mut body = res.bytes_stream();
                while let Some(chunk) = body.next().await {
                    let chunk = chunk.map_err(|e| S3Error { status: None, code: String::new(), message: e.to_string(), source: None })?;
                    bytes.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    out.write_all(&chunk).await.map_err(io_err)?;
                }
                out.flush().await.map_err(io_err)?;
            }
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            (ctx.report)(Progress { done: n, total: Some(total), bytes: Some(bytes.load(Ordering::Relaxed)), current: Some(key) });
            Ok(())
        }
    })
    .await
}

/// Fichiers locaux à envoyer (dossiers parcourus) → clé relative.
fn local_files(paths: &[PathBuf]) -> std::io::Result<Vec<(PathBuf, String, u64)>> {
    fn walk(file: &Path, relative: String, out: &mut Vec<(PathBuf, String, u64)>) -> std::io::Result<()> {
        let meta = std::fs::metadata(file)?;
        if meta.is_dir() {
            let mut children: Vec<_> = std::fs::read_dir(file)?.filter_map(Result::ok).collect();
            children.sort_by_key(|c| c.file_name());
            for child in children {
                walk(&child.path(), format!("{relative}/{}", child.file_name().to_string_lossy()), out)?;
            }
        } else if meta.is_file() {
            out.push((file.to_path_buf(), relative, meta.len()));
        }
        Ok(())
    }
    let mut out = vec![];
    for p in paths {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        walk(p, name, &mut out)?;
    }
    Ok(out)
}

/// Envoie des fichiers et dossiers locaux dans `prefix` (envoi en plusieurs parties au-delà
/// de 8 Mo).
pub async fn upload(ctx: &OpContext, prefix: &str, paths: &[PathBuf]) -> Result<(), S3Error> {
    let files = local_files(paths).map_err(io_err)?;
    let total = files.len();
    let done = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    (ctx.report)(Progress { done: 0, total: Some(total), bytes: Some(0), current: None });
    pool(files, 3, &ctx.cancel, |(file, relative, size)| {
        let (done, bytes) = (&done, &bytes);
        async move {
            let key = format!("{prefix}{relative}");
            upload_file(ctx, &key, &file, size).await?;
            let b = bytes.fetch_add(size, Ordering::Relaxed) + size;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            (ctx.report)(Progress { done: n, total: Some(total), bytes: Some(b), current: Some(key) });
            Ok(())
        }
    })
    .await
}

async fn upload_file(ctx: &OpContext, key: &str, file: &Path, size: u64) -> Result<(), S3Error> {
    let timeout = Duration::from_secs(600);
    if size <= PART_SIZE as u64 {
        let data = tokio::fs::read(file).await.map_err(io_err)?;
        ctx.client.request("PUT", Some(&ctx.bucket), Some(key), &[], &[], Some(Bytes::from(data)), timeout).await?;
        return Ok(());
    }
    let res = ctx.client.request("POST", Some(&ctx.bucket), Some(key), &[("uploads", String::new())], &[], None, timeout).await?;
    #[derive(Deserialize, Default)]
    #[serde(rename_all = "PascalCase", default)]
    struct Init {
        upload_id: String,
    }
    let init: Init = xml(&S3Client::text(res).await?)?;
    let id = init.upload_id;
    let result = async {
        let parts = size.div_ceil(PART_SIZE as u64) as usize;
        let etags: Vec<parking_lot::Mutex<String>> = (0..parts).map(|_| parking_lot::Mutex::new(String::new())).collect();
        pool((0..parts).collect(), 4, &ctx.cancel, |i| {
            let (id, etags) = (&id, &etags);
            async move {
                use tokio::io::{AsyncReadExt, AsyncSeekExt};
                let mut f = tokio::fs::File::open(file).await.map_err(io_err)?;
                f.seek(std::io::SeekFrom::Start((i * PART_SIZE) as u64)).await.map_err(io_err)?;
                let mut buf = vec![0u8; PART_SIZE.min((size - (i * PART_SIZE) as u64) as usize)];
                f.read_exact(&mut buf).await.map_err(io_err)?;
                let q = [("partNumber", (i + 1).to_string()), ("uploadId", id.clone())];
                let res = ctx.client.request("PUT", Some(&ctx.bucket), Some(key), &q, &[], Some(Bytes::from(buf)), timeout).await?;
                *etags[i].lock() = header(&res, "etag").unwrap_or_default();
                Ok(())
            }
        })
        .await?;
        let body: String = etags
            .iter()
            .enumerate()
            .map(|(i, e)| format!("<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>", i + 1, e.lock().replace('&', "&amp;")))
            .collect();
        let xml_body = format!("<CompleteMultipartUpload>{body}</CompleteMultipartUpload>");
        let headers = [("content-type".to_string(), "application/xml".to_string())];
        let res = ctx.client.request("POST", Some(&ctx.bucket), Some(key), &[("uploadId", id.clone())], &headers, Some(Bytes::from(xml_body)), timeout).await?;
        // Erreur possible dans une réponse 200.
        let text = S3Client::text(res).await?;
        if text.contains("<Error>") { return Err(S3Error { status: Some(200), code: "UnknownError".into(), message: text, source: None }); }
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = ctx.client.request("DELETE", Some(&ctx.bucket), Some(key), &[("uploadId", id.clone())], &[], None, Duration::from_secs(30)).await;
    }
    result
}

/// Supprime des fichiers et des dossiers (récursif).
pub async fn remove(ctx: &OpContext, entries: &[S3Entry]) -> Result<(), S3Error> {
    let mut keys = expand(ctx, entries).await?;
    // Dossier vidé entre-temps : on supprime quand même son éventuel marqueur.
    for e in entries {
        if e.is_folder() && !keys.iter().any(|(k, _, _)| k.starts_with(&e.key)) { keys.push((e.key.clone(), 0, String::new())); }
    }
    let total = keys.len();
    let done = AtomicUsize::new(0);
    (ctx.report)(Progress { done: 0, total: Some(total), ..Default::default() });
    pool(keys, 8, &ctx.cancel, |(key, _, _)| {
        let done = &done;
        async move {
            ctx.client.send("DELETE", Some(&ctx.bucket), Some(&key), &[]).await?;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            (ctx.report)(Progress { done: n, total: Some(total), current: Some(key), ..Default::default() });
            Ok(())
        }
    })
    .await
}

/// Renomme un fichier ou un dossier dans son dossier parent : copie puis suppression. Les
/// copies sont toutes faites avant la moindre suppression.
pub async fn rename(ctx: &OpContext, entry: &S3Entry, new_name: &str) -> Result<(), S3Error> {
    let name = new_name.trim();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(S3Error { status: None, code: "Invalid".into(), message: String::new(), source: Some(SourceError::invalid("Nom invalide (vide ou contenant « / »).")) });
    }
    let target = format!("{}{name}{}", parent_of(&entry.key), if entry.is_folder() { "/" } else { "" });
    if target == entry.key { return Ok(()); }
    let keys: Vec<String> = if entry.is_folder() {
        all_keys(&ctx.client, &ctx.bucket, &entry.key, &ctx.cancel).await?.into_iter().map(|(k, _)| k).collect()
    } else {
        vec![entry.key.clone()]
    };
    let total = keys.len() * 2;
    let done = AtomicUsize::new(0);
    (ctx.report)(Progress { done: 0, total: Some(total), ..Default::default() });
    pool(keys.clone(), 8, &ctx.cancel, |key| {
        let (done, target) = (&done, &target);
        async move {
            let source = format!("{}/{}", ctx.bucket, client::uri_encode(&key, true));
            let headers = [("x-amz-copy-source".to_string(), source)];
            let res = ctx
                .client
                .request("PUT", Some(&ctx.bucket), Some(&format!("{target}{}", &key[entry.key.len()..])), &[], &headers, None, Duration::from_secs(600))
                .await?;
            let text = S3Client::text(res).await?;
            if text.contains("<Error>") { return Err(S3Error { status: Some(200), code: "UnknownError".into(), message: text, source: None }); }
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            (ctx.report)(Progress { done: n, total: Some(total), current: Some(key), ..Default::default() });
            Ok(())
        }
    })
    .await?;
    pool(keys, 8, &ctx.cancel, |key| {
        let done = &done;
        async move {
            ctx.client.send("DELETE", Some(&ctx.bucket), Some(&key), &[]).await?;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            (ctx.report)(Progress { done: n, total: Some(total), current: Some(key), ..Default::default() });
            Ok(())
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemins_locaux() {
        let d = Path::new("/d");
        assert_eq!(local_path(d, "a/b:c?.txt"), Path::new("/d/a/b_c_.txt"));
        assert_eq!(local_path(d, "../../etc/passwd"), Path::new("/d/_/_/etc/passwd"));
        assert_eq!(local_path(d, "x//y/"), Path::new("/d/x/y"));
        assert_eq!(parent_of("a/b/c.txt"), "a/b/");
        assert_eq!(parent_of("a/b/"), "a/");
        assert_eq!(parent_of("c.txt"), "");
        assert_eq!(base_name("a/b/"), "b");
    }

    #[test]
    fn gzip_tronque() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all("compressé\n".repeat(1000).as_bytes()).unwrap();
        let full = enc.finish().unwrap();
        assert_eq!(gunzip_head(&full, 1_000_000).len(), "compressé\n".len() * 1000);
        assert!(!gunzip_head(&full[..full.len() / 2], 1_000_000).is_empty());
        assert_eq!(gunzip_head(&full, 10).len(), 10);
    }
}
