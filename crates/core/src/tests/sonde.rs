//! Sonde de bout en bout : vraie sonde (HTTPS, jeton) sur un port libre, vrai client du poste.
//! Plusieurs postes abonnés, limites, reprise après coupure, jeton renouvelé, sécurité.

use super::support::{assert_match, call, test_app};
use crate::config::{SondeConnection, Tls};
use crate::sonde::client;
use crate::sonde::config::{self as sc, Init};
use crate::sonde::server::{self, Running, Timing};
use crate::streams::file::FileTiming;
use crate::streams::{StreamEvent, StreamOptions, StreamState};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn timing() -> Timing {
    Timing {
        ping: Duration::from_millis(300),
        reload: Duration::from_millis(50),
        status: Duration::from_millis(50),
        file: FileTiming { poll: Duration::from_millis(20), idle_flush: Duration::from_millis(200), batch_lines: 1_000 },
    }
}

fn stream_timing() -> crate::streams::Timing {
    crate::streams::Timing { batch: Duration::from_millis(10), batch_lines: 500, retry_delays: vec![Duration::from_millis(50), Duration::from_millis(100)], idle_timeout: Duration::from_secs(5) }
}

struct Probe {
    dir: tempfile::TempDir,
    init: Init,
    logs: PathBuf,
}

impl Probe {
    fn new() -> Probe {
        let dir = tempfile::tempdir().unwrap();
        let init = sc::init(&dir.path().join("sonde"), Some("127.0.0.1:0"), Some("srv-test")).unwrap();
        let logs = dir.path().join("logs");
        std::fs::create_dir(&logs).unwrap();
        std::fs::write(logs.join("api.log"), "un\ndeux\ntrois\n").unwrap();
        std::fs::write(logs.join("batch-1.log"), "b1\n").unwrap();
        std::fs::write(logs.join("secret.txt"), "mot de passe\n").unwrap();
        let probe = Probe { dir, init, logs };
        probe.set_sources(&format!(
            "  - id: api\n    nom: API\n    fichier: {}\n  - id: batch\n    motif: {}/batch-*.log\n",
            probe.logs.join("api.log").display(),
            probe.logs.display()
        ));
        probe
    }

    fn set_sources(&self, sources: &str) {
        let text = std::fs::read_to_string(&self.init.config).unwrap();
        let start = text.find("sources:").unwrap();
        let end = text.find("limites:").unwrap();
        let next = format!("{}sources:\n{sources}{}", &text[..start], &text[end..]);
        std::fs::write(&self.init.config, next).unwrap();
    }

    fn set(&self, from: &str, to: &str) {
        let text = std::fs::read_to_string(&self.init.config).unwrap();
        assert!(text.contains(from), "{from}");
        std::fs::write(&self.init.config, text.replace(from, to)).unwrap();
    }

    async fn start(&self) -> Running { server::start(&self.init.config, timing()).await.unwrap() }

    async fn start_on(&self, port: u16) -> Running {
        self.set("ecoute: 127.0.0.1:0", &format!("ecoute: 127.0.0.1:{port}"));
        server::start(&self.init.config, timing()).await.unwrap()
    }

    fn connection(&self, running: &Running) -> SondeConnection {
        SondeConnection {
            id: "sonde-test".into(),
            name: "srv-test".into(),
            url: format!("https://127.0.0.1:{}", running.addr.port()),
            tls: Tls { insecure: false, ca_file: None, fingerprint: Some(self.init.fingerprint.clone()) },
        }
    }

    fn append(&self, name: &str, text: &str) {
        std::fs::OpenOptions::new().append(true).open(self.logs.join(name)).unwrap().write_all(text.as_bytes()).unwrap();
    }
}

/// Événements reçus par un abonnement côté poste.
#[derive(Default)]
struct Received {
    lines: Vec<String>,
    notices: Vec<String>,
    states: Vec<(StreamState, Option<String>)>,
}

fn subscribe(c: &SondeConnection, token: &str, path: &[&str], options: StreamOptions) -> (Arc<Mutex<Received>>, CancellationToken) {
    let got: Arc<Mutex<Received>> = Arc::default();
    let cancel = CancellationToken::new();
    let g = got.clone();
    let emit: crate::streams::Emit = Arc::new(move |e: StreamEvent| {
        let mut g = g.lock();
        match e {
            StreamEvent::Lines { lines } => g.lines.extend(lines.into_iter().map(|l| l.text)),
            StreamEvent::Notice { message } => g.notices.push(message),
            StreamEvent::Status { state, message, .. } => g.states.push((state, message)),
        }
    });
    let path: Vec<String> = path.iter().map(|s| s.to_string()).collect();
    client::spawn(c.clone(), token.into(), &path, options, emit, stream_timing(), cancel.clone()).unwrap();
    (got, cancel)
}

async fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..400 {
        if ok() { return; }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("délai dépassé : {what}");
}

fn follow(tail: u64) -> StreamOptions { StreamOptions { follow: true, tail_lines: Some(tail), ..Default::default() } }

async fn raw(c: &SondeConnection, token: &str, path: &str) -> (u16, Value) {
    let response = crate::net::http_client(&c.tls).unwrap().get(format!("{}{path}", c.url)).bearer_auth(token).send().await.unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn connexion_parcours_et_securite() {
    let probe = Probe::new();
    let running = probe.start().await;
    let c = probe.connection(&running);
    let token = &probe.init.token;

    // Premier contact : empreinte présentée = celle affichée par `sonde init`.
    assert_eq!(client::fetch_fingerprint(&c.url).await.unwrap(), probe.init.fingerprint);
    let summary = client::test(&c, token).await.unwrap();
    assert!(summary.contains("srv-test") && summary.contains("2 source(s)"), "{summary}");

    // Arbre : sources, puis fichiers d'un motif.
    let nodes = serde_json::to_value(client::browse(&c, token, &[]).await.unwrap()).unwrap();
    assert_match(&nodes, &json!([
        { "id": "api", "kind": "source", "expandable": false, "health": "ok", "state": "API", "size": 14 },
        { "id": "batch", "kind": "pattern", "expandable": true, "health": "ok" },
    ]));
    let files = serde_json::to_value(client::browse(&c, token, &["batch".into()]).await.unwrap()).unwrap();
    assert_match(&files, &json!([{ "id": "batch-1.log", "kind": "file", "size": 3 }]));

    // Certificat différent de celui approuvé : refusé.
    let other = SondeConnection { tls: Tls { fingerprint: Some(format!("SHA256:{}", vec!["00"; 32].join(":"))), ..c.tls.clone() }, ..c.clone() };
    let e = client::test(&other, token).await.unwrap_err();
    assert_eq!(e.code, crate::error::ErrorCode::Tls, "{e:?}");

    // Jeton invalide ; chemins hors liste blanche ; curseur invalide.
    let e = client::test(&c, "osk_faux").await.unwrap_err();
    assert_eq!(e.code, crate::error::ErrorCode::Unauthorized);
    assert_eq!(raw(&c, token, "/v1/sources/inconnue/flux").await.0, 404);
    for evil in ["..%2Fsecret.txt", "..%2F..%2Fsecret.txt", "secret.txt", "batch-1.log%2F..%2F..%2Fsecret.txt"] {
        let (status, body) = raw(&c, token, &format!("/v1/sources/batch/flux?fichier={evil}")).await;
        assert_eq!(status, 404, "{evil} : {body}");
    }
    assert_eq!(raw(&c, token, "/v1/sources/api/flux?fichier=api.log").await.0, 400);
    assert_eq!(raw(&c, token, "/v1/sources/api/flux?reprise=nimporte").await.0, 400);
    assert_eq!(raw(&c, token, "/v1/autre").await.0, 404);

    // Blocage après 10 échecs (pour cette adresse), même avec le bon jeton ensuite.
    // Un échec déjà compté (jeton invalide ci-dessus) : 8 de plus, puis le 10e bloque.
    for _ in 0..8 { assert_eq!(raw(&c, "osk_faux", "/v1/info").await.0, 401); }
    assert_eq!(raw(&c, token, "/v1/info").await.0, 200, "encore sous la limite");
    assert_eq!(raw(&c, "osk_faux", "/v1/info").await.0, 401);
    let (status, body) = raw(&c, token, "/v1/info").await;
    assert_eq!(status, 429);
    assert!(body["erreur"].as_str().unwrap().contains("Trop d'échecs"));
    running.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn plusieurs_postes_abonnes() {
    let probe = Probe::new();
    probe.set("flux_par_poste: 10", "flux_par_poste: 5");
    let running = probe.start().await;
    let c = probe.connection(&running);
    let token = &probe.init.token;

    // Cinq abonnements aux options différentes, sur la même source ou non.
    let (a, _ca) = subscribe(&c, token, &["api"], follow(10));
    let (b, _cb) = subscribe(&c, token, &["api"], follow(1));
    let (h, _ch) = subscribe(&c, token, &["api"], StreamOptions { follow: false, tail_lines: Some(2), ..Default::default() });
    let (m, _cm) = subscribe(&c, token, &["batch"], follow(10));
    let (f, _cf) = subscribe(&c, token, &["batch", "batch-1.log"], StreamOptions { follow: false, ..Default::default() });
    wait("historiques", || a.lock().lines.len() == 3 && b.lock().lines.len() == 1 && m.lock().lines.len() == 1).await;
    wait("historique seul terminé", || h.lock().states.iter().any(|s| s.0 == StreamState::Ended)).await;
    assert_eq!(h.lock().lines, ["deux", "trois"]);
    wait("fichier du motif", || f.lock().lines == ["b1"]).await;
    assert!(a.lock().states.iter().any(|s| s.0 == StreamState::Live));

    // Chacun reçoit les nouvelles lignes, indépendamment.
    probe.append("api.log", "quatre\n");
    wait("diffusion", || a.lock().lines.last().map(String::as_str) == Some("quatre") && b.lock().lines == ["trois", "quatre"]).await;
    // Nouveau fichier dans le motif : bascule pour l'abonné au motif.
    probe.append("batch-1.log", "b2\n");
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::write(probe.logs.join("batch-2.log"), "c1\n").unwrap();
    wait("bascule", || m.lock().lines == ["b1", "b2", "c1"]).await;
    assert!(m.lock().notices.contains(&"Nouveau fichier : batch-2.log".to_string()));

    // État des abonnés (sonde statut) : postes identifiés, lignes comptées.
    wait("statut", || running.subscribers().len() == 3).await;
    let subs = running.subscribers();
    assert!(subs.iter().all(|s| s.poste == client::poste() && s.ip == "127.0.0.1"));
    assert_eq!(subs.iter().filter(|s| s.source == "api").count(), 2);
    let status = server::read_status(&probe.dir.path().join("sonde")).unwrap();
    wait("fichier de statut", || server::read_status(&probe.dir.path().join("sonde")).unwrap()["abonnes"].as_array().unwrap().len() == 3).await;
    assert_eq!(status["nom"], "srv-test");

    // Limite par poste (5) : 3 flux en cours + 2 de plus, le 6e est refusé.
    let (_x, _cx) = subscribe(&c, token, &["api"], follow(0));
    let (_y, _cy) = subscribe(&c, token, &["api"], follow(0));
    wait("5 abonnés", || running.subscribers().len() == 5).await;
    let (z, _cz) = subscribe(&c, token, &["api"], follow(0));
    wait("refus", || z.lock().states.iter().any(|s| s.0 == StreamState::Error)).await;
    let message = z.lock().states.last().unwrap().1.clone().unwrap();
    assert!(message.contains("Trop de flux ouverts depuis ce poste"), "{message}");

    // Un poste qui part ne gêne pas les autres.
    _ca.cancel();
    wait("départ", || running.subscribers().len() == 4).await;
    probe.append("api.log", "cinq\n");
    wait("après départ", || b.lock().lines.last().map(String::as_str) == Some("cinq")).await;

    // Source retirée : ses abonnés sont prévenus ; jeton renouvelé : tous fermés.
    probe.set_sources(&format!("  - id: api\n    nom: API\n    fichier: {}\n", probe.logs.join("api.log").display()));
    wait("source retirée", || m.lock().states.iter().any(|s| s.0 == StreamState::Ended)).await;
    assert!(m.lock().states.last().unwrap().1.as_deref().unwrap().contains("Source retirée"));
    let new_token = sc::renew_token(&probe.init.config).unwrap();
    wait("jeton renouvelé", || b.lock().states.iter().any(|s| s.0 == StreamState::Ended)).await;
    assert!(b.lock().states.last().unwrap().1.as_deref().unwrap().contains("Jeton"));
    wait("plus d'abonnés", || running.subscribers().is_empty()).await;
    assert_eq!(client::test(&c, token).await.unwrap_err().code, crate::error::ErrorCode::Unauthorized);
    assert!(client::test(&c, &new_token).await.is_ok());
    running.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn reprise_apres_coupure_sans_perte_ni_doublon() {
    let probe = Probe::new();
    let first = probe.start().await;
    let port = first.addr.port();
    drop(first);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let running = probe.start_on(port).await;
    let c = probe.connection(&running);
    let token = probe.init.token.clone();
    let (got, _cancel) = subscribe(&c, &token, &["api"], follow(10));
    wait("historique", || got.lock().lines.len() == 3).await;
    probe.append("api.log", "quatre\n");
    wait("quatre", || got.lock().lines.len() == 4).await;

    // Coupure : la sonde s'arrête ; des lignes arrivent pendant ce temps.
    running.stop();
    drop(running);
    wait("reconnexion", || got.lock().states.iter().any(|s| s.0 == StreamState::Reconnecting)).await;
    probe.append("api.log", "cinq\nsix\n");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let running = server::start(&probe.init.config, timing()).await.unwrap();
    assert_eq!(running.addr.port(), port);
    wait("reprise", || got.lock().lines.len() == 6).await;
    probe.append("api.log", "sept\n");
    wait("suite", || got.lock().lines.len() == 7).await;
    assert_eq!(got.lock().lines, ["un", "deux", "trois", "quatre", "cinq", "six", "sept"]);
    running.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn ndjson_gzip_et_ping() {
    let probe = Probe::new();
    let running = probe.start().await;
    let c = probe.connection(&running);
    let client = crate::net::http_client(&c.tls).unwrap();
    // Corps compressé, lignes NDJSON avec curseur, puis ping quand rien ne bouge.
    let mut response = client.get(format!("{}/v1/sources/api/flux?suivi=1&lignes=2", c.url))
        .bearer_auth(&probe.init.token).header("accept-encoding", "gzip").send().await.unwrap();
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert!(response.headers()["content-type"].to_str().unwrap().starts_with("application/x-ndjson"));
    let mut decoder = flate2::write::GzDecoder::new(Vec::new());
    let mut text = String::new();
    while !text.contains("\"ping\"") {
        let chunk = tokio::time::timeout(Duration::from_secs(5), response.chunk()).await.unwrap().unwrap().unwrap();
        decoder.write_all(&chunk).unwrap();
        decoder.flush().unwrap();
        text.push_str(&String::from_utf8(std::mem::take(decoder.get_mut())).unwrap());
    }
    let events: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let lines = events.iter().find(|e| e["type"] == "lines").unwrap();
    assert_eq!(lines["lines"], json!([{ "text": "deux" }, { "text": "trois" }]));
    assert!(crate::streams::file::Cursor::decode(lines["curseur"].as_str().unwrap()).is_some());
    assert!(events.iter().any(|e| e["type"] == "status" && e["state"] == "live"));
    running.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn sonde_dans_l_application() {
    let probe = Probe::new();
    let running = probe.start().await;
    let url = format!("https://127.0.0.1:{}", running.addr.port());
    let t = test_app("# conf\n", true).await;
    let app = &t.app;
    // Connexion enregistrée comme les autres (jeton chiffré à part, empreinte dans config.yaml).
    let id = app.connections.save(&json!({ "type": "sonde", "name": "srv-test", "url": url, "tls": { "insecure": false, "fingerprint": probe.init.fingerprint }, "secret": probe.init.token })).unwrap();
    let yaml = t.read("config.yaml");
    assert!(yaml.contains("type: sonde") && yaml.contains("fingerprint: SHA256:") && !yaml.contains(&probe.init.token), "{yaml}");
    // Outils MCP : parcourir, lire, lien de partage.
    let sources = call(app, "sources_parcourir", json!({ "connexion": id })).await;
    assert_match(&sources.value, &json!([{ "id": "api" }, { "id": "batch" }]));
    let read = call(app, "logs_lire", json!({ "connexion": id, "chemin": ["api"], "lignes": 2 })).await;
    assert!(!read.error, "{}", read.text);
    assert_match(&read.value, &json!({ "etat": "ended", "profil": "sonde/srv-test/api", "recues": 2 }));
    let link = call(app, "lien_creer", json!({ "connexion": id, "chemin": ["batch", "batch-1.log"] })).await;
    let opened = call(app, "lien_ouvrir", json!({ "lien": link.value })).await;
    assert_match(&opened.value, &json!({ "connexion": id, "type": "sonde", "chemin": ["batch", "batch-1.log"] }));
    running.stop();
}
