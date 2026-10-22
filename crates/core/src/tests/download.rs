//! Téléchargement de l'historique complet (« Télécharger… ») : contenu exact, formats, filtres,
//! encodage, erreurs de source, assemblage, troncature.

use super::support::{TestApp, test_app_with};
use crate::api::{Caller, call};
use crate::testing::fake_rancher::{self, FAKE_TOKEN, HEALTHY};
use crate::ui::{FileFilter, Ui};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Boîte « Enregistrer sous » scriptée.
#[derive(Default)]
struct SaveUi {
    answers: Mutex<Vec<PathBuf>>,
}

#[async_trait::async_trait]
impl Ui for SaveUi {
    async fn save_dialog(&self, _: &str, _: PathBuf, _: Vec<FileFilter>) -> Option<PathBuf> { self.answers.lock().pop() }
}

async fn setup() -> (TestApp, Arc<SaveUi>) {
    let ui = Arc::new(SaveUi::default());
    let t = test_app_with("# conf\n", true, ui.clone()).await;
    (t, ui)
}

/// Télécharge vers `name` (dans le dossier de test) ; renvoie la réponse et le fichier écrit.
async fn download(t: &TestApp, ui: &SaveUi, name: &str, request: Value) -> (Result<Value, String>, String) {
    let out = t.path(name);
    ui.answers.lock().push(out.clone());
    let caller = Caller { window: "main".into() };
    let result = tokio::time::timeout(Duration::from_secs(60), call(&t.app, &caller, "export:downloadLogs", vec![request])).await.expect("téléchargement bloqué");
    (result, std::fs::read_to_string(&out).unwrap_or_default())
}

fn local(path: &std::path::Path) -> Value { json!({ "connectionId": "local", "path": [path.to_string_lossy()] }) }

#[tokio::test(flavor = "multi_thread")]
async fn fichier_complet_texte_et_ndjson() {
    let (t, ui) = setup().await;
    let file = t.path("app.log");
    let mut text = String::new();
    for i in 0..800 {
        text.push_str(&format!("2026-10-22 10:{:02}:{:02} {} message {i}\n", (i / 60) % 60, i % 60, if i % 10 == 0 { "ERROR" } else { "INFO" }));
    }
    std::fs::write(&file, &text).unwrap();

    let (r, written) = download(&t, &ui, "tout.log", json!({ "defaultName": "app", "filtered": false, "sources": [local(&file)] })).await;
    let r = r.unwrap();
    assert_eq!(written, text, "texte brut : fichier identique à la source");
    assert_eq!((r["lines"].as_u64(), r["truncated"].as_bool()), (Some(800), Some(false)));

    let (r, written) = download(&t, &ui, "tout.ndjson", json!({ "defaultName": "app", "filtered": false, "sources": [local(&file)] })).await;
    assert_eq!(r.unwrap()["lines"], 800);
    let lines: Vec<Value> = written.lines().map(|l| serde_json::from_str(l).expect("NDJSON valide")).collect();
    assert_eq!(lines.len(), 800);
    assert_eq!(lines[799]["message"], "2026-10-22 10:13:19 INFO message 799");

    // Filtré : recherche et niveau.
    let request = json!({ "defaultName": "app", "filtered": true, "search": { "text": "message 4", "caseSensitive": false, "regex": false }, "levels": ["error"], "sources": [local(&file)] });
    let (r, written) = download(&t, &ui, "filtre.log", request).await;
    let expected: Vec<&str> = text.lines().filter(|l| l.contains("message 4") && l.contains("ERROR")).collect();
    assert_eq!(written.lines().collect::<Vec<_>>(), expected);
    assert_eq!(r.unwrap()["lines"].as_u64().unwrap() as usize, expected.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn fichier_encodage_windows_1252() {
    let (t, ui) = setup().await;
    let file = t.path("ancien.log");
    // « Données reçues : élément créé » en Windows-1252.
    let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode("Données reçues : élément créé\n");
    std::fs::write(&file, &bytes).unwrap();
    let path = file.to_string_lossy().to_string();

    // Encodage choisi dans l'onglet (transmis par l'interface).
    let source = json!({ "connectionId": "local", "path": [path], "encoding": "windows-1252" });
    let (r, written) = download(&t, &ui, "a.log", json!({ "defaultName": "x", "filtered": false, "sources": [source] })).await;
    r.unwrap();
    assert_eq!(written, "Données reçues : élément créé\n");

    // Encodage mémorisé pour ce fichier (onglet rouvert sans le préciser).
    t.app.files.set_encoding(&path, Some("windows-1252")).unwrap();
    let (r, written) = download(&t, &ui, "b.log", json!({ "defaultName": "x", "filtered": false, "sources": [local(&file)] })).await;
    r.unwrap();
    assert_eq!(written, "Données reçues : élément créé\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn source_en_erreur_signalee() {
    let (t, ui) = setup().await;
    // Fichier absent : erreur lisible, pas un fichier vide « 0 ligne ».
    let (r, _) = download(&t, &ui, "absent.log", json!({ "defaultName": "x", "filtered": false, "sources": [local(&t.path("absent.log"))] })).await;
    let error = r.expect_err("fichier absent : erreur attendue");
    assert!(error.contains("absent.log") || error.to_lowercase().contains("introuvable"), "{error}");

    // Pod disparu (Rancher) : idem.
    let rancher = fake_rancher::start(0).await;
    let caller = Caller { window: "main".into() };
    let draft = json!({ "type": "rancher", "name": "R", "url": rancher.url, "secret": FAKE_TOKEN, "tls": { "insecure": true }, "clusters": [{ "id": "c-prod1", "name": "prod" }] });
    let id = call(&t.app, &caller, "connections:save", vec![draft]).await.unwrap();
    let gone = json!({ "connectionId": id, "path": ["c-prod1", "app-commandes", "api-commandes-disparu-zzzzz", "app"] });
    let (r, _) = download(&t, &ui, "pod.log", json!({ "defaultName": "x", "filtered": false, "sources": [gone] })).await;
    assert!(r.is_err(), "pod disparu : erreur attendue, obtenu {r:?}");

    // Une source en erreur dans un assemblage : le téléchargement le dit (pas de fichier partiel muet).
    let ok = json!({ "connectionId": id, "path": ["c-prod1", "app-commandes", HEALTHY, "app"], "label": "api" });
    let gone = json!({ "connectionId": id, "path": ["c-prod1", "app-commandes", "api-commandes-disparu-zzzzz", "app"], "label": "parti" });
    let (r, _) = download(&t, &ui, "assemblage.log", json!({ "defaultName": "x", "filtered": false, "sources": [ok, gone] })).await;
    let error = r.expect_err("assemblage avec une source en erreur");
    assert!(error.contains("parti"), "la source en cause est nommée : {error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn assemblage_garde_les_lignes_de_suite() {
    let (t, ui) = setup().await;
    let (a, b) = (t.path("a.log"), t.path("b.log"));
    std::fs::write(&a, "2026-10-22T10:00:00.000Z ERROR échec A\n    at a.Foo(Foo.java:1)\n    at a.Bar(Bar.java:2)\n2026-10-22T10:00:03.000Z INFO fin A\n").unwrap();
    std::fs::write(&b, "2026-10-22T10:00:01.000Z INFO début B\n2026-10-22T10:00:02.000Z WARN milieu B\n").unwrap();
    let sources = json!([{ "connectionId": "local", "path": [a.to_string_lossy()], "label": "A" }, { "connectionId": "local", "path": [b.to_string_lossy()], "label": "B" }]);
    let (r, written) = download(&t, &ui, "fusion.log", json!({ "defaultName": "x", "filtered": false, "sources": sources })).await;
    r.unwrap();
    assert_eq!(
        written.lines().collect::<Vec<_>>(),
        vec![
            "[A] 2026-10-22T10:00:00.000Z ERROR échec A",
            "[A]     at a.Foo(Foo.java:1)",
            "[A]     at a.Bar(Bar.java:2)",
            "[B] 2026-10-22T10:00:01.000Z INFO début B",
            "[B] 2026-10-22T10:00:02.000Z WARN milieu B",
            "[A] 2026-10-22T10:00:03.000Z INFO fin A",
        ],
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn troncature_signalee() {
    let (t, ui) = setup().await;
    let file = t.path("gros.log");
    let total = crate::api::DOWNLOAD_CAP + 50;
    std::fs::write(&file, (0..total).map(|i| format!("ligne {i}\n")).collect::<String>()).unwrap();
    let (r, written) = download(&t, &ui, "gros.log.txt", json!({ "defaultName": "x", "filtered": false, "sources": [local(&file)] })).await;
    let r = r.unwrap();
    assert_eq!(r["truncated"], true, "source unique au-delà de la limite : troncature signalée");
    assert_eq!(written.lines().count(), crate::api::DOWNLOAD_CAP);
    assert_eq!(written.lines().last(), Some(format!("ligne {}", total - 1).as_str()), "les lignes les plus récentes sont gardées");
}
