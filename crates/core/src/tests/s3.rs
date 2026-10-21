//! Tests d'intégration contre un S3 compatible (scripts/s3-test-server.sh : moto) :
//!   S3_TEST_URL=http://localhost:19000 cargo test -p ocekour-core s3

use crate::config::{Connection, S3Connection, connection_schema};
use crate::s3::client::S3Client;
use crate::s3::{self, OpContext, PREVIEW_BYTES, Progress, S3Entry};
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn connection(url: &str, buckets: &[&str]) -> S3Connection {
    match connection_schema()
        .parse_into::<Connection>(&json!({ "id": "s3-test", "type": "s3", "name": "test", "url": url, "accessKeyId": "test", "buckets": buckets }))
        .unwrap()
    {
        Connection::S3(c) => c,
        _ => unreachable!(),
    }
}

fn ctx(client: &Arc<S3Client>, bucket: &str) -> (OpContext, Arc<Mutex<Vec<Progress>>>) {
    let reports = Arc::new(Mutex::new(vec![]));
    let sink = reports.clone();
    (
        OpContext { client: client.clone(), bucket: bucket.into(), cancel: CancellationToken::new(), report: Arc::new(move |p| sink.lock().push(p)) },
        reports,
    )
}

async fn names(client: &S3Client, bucket: &str, prefix: &str) -> Vec<String> {
    s3::list_folder(client, bucket, prefix, "", None).await.unwrap().entries.into_iter().map(|e| e.name).collect()
}

fn entry(key: &str) -> S3Entry { S3Entry::from_key(key) }

#[tokio::test]
async fn s3_serveur_compatible() {
    let Ok(url) = std::env::var("S3_TEST_URL") else { return };
    let bucket = format!("ocekour-{:x}", crate::text::now_ms());
    let conn = connection(&url, &["manuel"]);
    let client = Arc::new(S3Client::new(&conn, "test-secret").unwrap());
    let t = Duration::from_secs(30);
    let put = |key: &str, body: Vec<u8>| {
        let (client, bucket, key) = (client.clone(), bucket.clone(), key.to_string());
        async move { client.request("PUT", Some(&bucket), Some(&key), &[], &[], Some(Bytes::from(body)), t).await.unwrap(); }
    };
    client.request("PUT", Some(&bucket), None, &[], &[], None, t).await.unwrap();
    put("logs/2026/app.log", b"ligne 1\nligne 2\n".to_vec()).await;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, "compressé\n".repeat(3).as_bytes()).unwrap();
    put("logs/2026/app.log.gz", gz.finish().unwrap()).await;
    put("logs/readme.txt", b"bonjour".to_vec()).await;
    put("image.png", hex::decode("89504e470d0a1a0a").unwrap()).await;
    put("binaire.bin", vec![0, 1, 2, 3]).await;
    put("gros.txt", vec![b'a'; PREVIEW_BYTES + 10]).await;
    put("vide/", vec![]).await;

    // Test de connexion et buckets.
    let summary = s3::test(&conn, "test-secret").await.unwrap();
    assert!(regex::Regex::new(r"^\d+ bucket\(s\) visible\(s\)$").unwrap().is_match(&summary), "{summary}");
    let buckets: Vec<String> = s3::list_buckets(&conn, "test-secret").await.unwrap().into_iter().map(|b| b.id).collect();
    assert!(buckets.contains(&bucket) && buckets.contains(&"manuel".to_string()));

    // Dossiers puis fichiers, marqueur ignoré, recherche par préfixe.
    let root = s3::list_folder(&client, &bucket, "", "", None).await.unwrap();
    let got: Vec<String> = root.entries.iter().map(|e| format!("{}:{}", e.kind, e.name)).collect();
    assert_eq!(got, vec!["folder:logs", "folder:vide", "file:binaire.bin", "file:gros.txt", "file:image.png"]);
    let big = root.entries.iter().find(|e| e.name == "gros.txt").unwrap();
    assert_eq!(big.size, Some((PREVIEW_BYTES + 10) as u64));
    assert!(big.modified.as_ref().unwrap().ends_with('Z'));
    assert!(names(&client, &bucket, "vide/").await.is_empty());
    assert_eq!(names(&client, &bucket, "logs/2026/").await, vec!["app.log", "app.log.gz"]);
    let search = s3::list_folder(&client, &bucket, "logs/", "re", None).await.unwrap();
    assert_eq!(search.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["readme.txt"]);

    // Pagination.
    let futures: Vec<_> = (0..1005).map(|i| put(&format!("page/f{i:04}"), b"x".to_vec())).collect();
    futures_util::future::join_all(futures).await;
    let first = s3::list_folder(&client, &bucket, "page/", "", None).await.unwrap();
    assert_eq!(first.entries.len(), 1000);
    let second = s3::list_folder(&client, &bucket, "page/", "", first.next.as_deref()).await.unwrap();
    assert_eq!(second.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["f1000", "f1001", "f1002", "f1003", "f1004"]);
    assert_eq!(s3::count_keys(&client, &bucket, "page/").await.unwrap(), 1005);

    // Aperçus.
    let p = s3::preview(&client, &bucket, "logs/2026/app.log").await.unwrap();
    assert_eq!((p.kind, p.text.as_deref(), p.truncated), ("text", Some("ligne 1\nligne 2\n"), false));
    let p = s3::preview(&client, &bucket, "logs/2026/app.log.gz").await.unwrap();
    assert_eq!((p.kind, p.text.unwrap(), p.gunzipped), ("text", "compressé\n".repeat(3), Some(true)));
    let p = s3::preview(&client, &bucket, "image.png").await.unwrap();
    assert_eq!(p.kind, "image");
    assert!(p.data_url.unwrap().starts_with("data:image/png;base64,"));
    assert_eq!(s3::preview(&client, &bucket, "binaire.bin").await.unwrap().kind, "binary");
    let p = s3::preview(&client, &bucket, "gros.txt").await.unwrap();
    assert_eq!((p.kind, p.truncated, p.size), ("text", true, (PREVIEW_BYTES + 10) as u64));
    assert_eq!(p.text.unwrap().len(), PREVIEW_BYTES);

    // Envoi d'un dossier local (dont un gros fichier en plusieurs parties) puis téléchargement.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("envoi");
    std::fs::create_dir_all(src.join("sous")).unwrap();
    std::fs::write(src.join("a.txt"), "A").unwrap();
    std::fs::write(src.join("sous").join("b.json"), r#"{"b":1}"#).unwrap();
    let large: Vec<u8> = (0..(17 * 1024 * 1024)).map(|i| (i % 251) as u8).collect();
    std::fs::write(src.join("sous").join("gros.bin"), &large).unwrap();
    let (up, reports) = ctx(&client, &bucket);
    s3::upload(&up, "recu/", &[src.clone()]).await.unwrap();
    let last = reports.lock().last().cloned().unwrap();
    assert_eq!((last.done, last.total), (3, Some(3)));
    assert_eq!(names(&client, &bucket, "recu/envoi/").await, vec!["sous", "a.txt"]);
    let dest = tmp.path().join("retour");
    s3::download(&ctx(&client, &bucket).0, &[entry("recu/envoi/")], None, Some(&dest)).await.unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("envoi").join("sous").join("b.json")).unwrap(), r#"{"b":1}"#);
    assert_eq!(std::fs::read(dest.join("envoi").join("sous").join("gros.bin")).unwrap(), large);
    let file = tmp.path().join("seul.log");
    let mut log = entry("logs/2026/app.log");
    log.size = Some(16);
    s3::download(&ctx(&client, &bucket).0, &[log], Some(&file), None).await.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "ligne 1\nligne 2\n");

    // Renommages.
    s3::rename(&ctx(&client, &bucket).0, &entry("logs/readme.txt"), "lisez-moi.txt").await.unwrap();
    assert_eq!(names(&client, &bucket, "logs/").await, vec!["2026", "lisez-moi.txt"]);
    s3::rename(&ctx(&client, &bucket).0, &entry("logs/2026/"), "archives").await.unwrap();
    assert_eq!(names(&client, &bucket, "logs/").await, vec!["archives", "lisez-moi.txt"]);
    assert_eq!(names(&client, &bucket, "logs/archives/").await, vec!["app.log", "app.log.gz"]);
    let e = s3::rename(&ctx(&client, &bucket).0, &entry("logs/lisez-moi.txt"), "a/b").await.unwrap_err();
    assert_eq!(s3::client::to_source_error(&e, "").code, crate::error::ErrorCode::Invalid);

    // Suppressions (récursif, dossier vide compris).
    let (c, reports) = ctx(&client, &bucket);
    s3::remove(&c, &[entry("logs/"), entry("vide/"), entry("binaire.bin")]).await.unwrap();
    let last = reports.lock().last().cloned().unwrap();
    assert_eq!((last.done, last.total), (5, Some(5)));
    assert_eq!(names(&client, &bucket, "").await, vec!["page", "recu", "gros.txt", "image.png"]);

    // Annulation : rien n'est fait.
    let (c, _) = ctx(&client, &bucket);
    c.cancel.cancel();
    assert!(s3::remove(&c, &[entry("page/")]).await.is_err());
    assert_eq!(s3::count_keys(&client, &bucket, "page/").await.unwrap(), 1005);
}
