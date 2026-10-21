use crate::config::Tls;
use crate::error::ErrorCode;
use crate::logs::LogLine;
use crate::rancher::{self, RancherTarget};
use crate::streams::lines::ts_key;
use crate::streams::rancher::{ContainerRef, spawn};
use crate::streams::{StreamEvent, StreamOptions, StreamState, Timing};
use crate::testing::fake_rancher::{self, CRASHING, FAKE_TOKEN, FakeRancher, HEALTHY};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

struct Ctx {
    rancher: FakeRancher,
    _dir: tempfile::TempDir,
    pem: String,
    der: String,
}

async fn setup() -> Ctx {
    let rancher = fake_rancher::start(0).await;
    let dir = tempfile::tempdir().unwrap();
    let pem = dir.path().join("ca.pem");
    let der = dir.path().join("ca.cer");
    std::fs::write(&pem, &rancher.ca_pem).unwrap();
    std::fs::write(&der, &rancher.ca_der).unwrap();
    Ctx { pem: pem.to_string_lossy().into(), der: der.to_string_lossy().into(), rancher, _dir: dir }
}

impl Ctx {
    fn target(&self) -> RancherTarget {
        RancherTarget { url: self.rancher.url.clone(), tls: Tls { insecure: false, ca_file: Some(self.pem.clone()) }, token: FAKE_TOKEN.into() }
    }
}

#[tokio::test]
async fn client_rancher() {
    let c = setup().await;
    assert_eq!(rancher::test_connection(&c.target()).await.unwrap(), "jdupont");
    let der = RancherTarget { tls: Tls { insecure: false, ca_file: Some(c.der.clone()) }, ..c.target() };
    assert_eq!(rancher::test_connection(&der).await.unwrap(), "jdupont");
    let insecure = RancherTarget { tls: Tls { insecure: true, ca_file: None }, ..c.target() };
    assert!(rancher::test_connection(&insecure).await.is_ok());
    let unknown = RancherTarget { tls: Tls::default(), ..c.target() };
    let e = rancher::test_connection(&unknown).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Tls);
    assert!(e.message.contains("Ignorer la vérification SSL"), "{}", e.message);
    let bad = RancherTarget { token: "token-x:bad".into(), ..c.target() };
    assert_eq!(rancher::test_connection(&bad).await.unwrap_err().code, ErrorCode::Unauthorized);
    let down = RancherTarget { url: "https://127.0.0.1:1".into(), ..c.target() };
    assert_eq!(rancher::test_connection(&down).await.unwrap_err().code, ErrorCode::Network);
    let clusters = rancher::list_clusters(&c.target()).await.unwrap();
    assert_eq!(clusters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["local", "prod", "recette"]);
}

#[tokio::test]
async fn parcours() {
    let c = setup().await;
    let t = c.target();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let ns = rancher::browse(&t, &s(&["c-prod1"])).await.unwrap();
    assert_eq!(ns.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), vec!["app-commandes", "default", "kube-system"]);
    assert_eq!((ns[0].kind, ns[0].expandable), ("namespace", true));
    let pods = rancher::browse(&t, &s(&["c-prod1", "app-commandes"])).await.unwrap();
    let got: Vec<_> = pods.iter().map(|p| (p.id.clone(), p.workload.clone().unwrap(), p.health.unwrap(), p.state.clone().unwrap(), p.restarts.unwrap())).collect();
    assert_eq!(got, vec![
        (CRASHING.into(), "api-commandes".into(), "error", "CrashLoopBackOff".into(), 5),
        (HEALTHY.into(), "api-commandes".into(), "ok", "Running".into(), 0),
        ("worker-0".into(), "worker".into(), "ok", "Running".into(), 2),
    ]);
    let containers = rancher::browse(&t, &s(&["c-prod1", "app-commandes", CRASHING])).await.unwrap();
    let got: Vec<_> = containers.iter().map(|c| (c.id.as_str(), c.init.unwrap(), c.state.clone().unwrap(), c.has_previous.unwrap(), c.expandable)).collect();
    assert_eq!(got, vec![
        ("migrations", true, "Completed".into(), false, false),
        ("app", false, "CrashLoopBackOff".into(), true, false),
        ("istio-proxy", false, "Running".into(), false, false),
    ]);
    assert_eq!(rancher::browse(&t, &s(&["local"])).await.unwrap_err().code, ErrorCode::Forbidden);
    assert_eq!(rancher::browse(&t, &s(&["c-prod1", "inconnu"])).await.unwrap_err().code, ErrorCode::Http);
    let bad = RancherTarget { token: "x".into(), tls: Tls { insecure: true, ca_file: None }, ..t.clone() };
    assert_eq!(rancher::browse(&bad, &s(&["c-prod1"])).await.unwrap_err().code, ErrorCode::Unauthorized);
    let no_ca = RancherTarget { tls: Tls::default(), ..t };
    assert_eq!(rancher::browse(&no_ca, &s(&["c-prod1"])).await.unwrap_err().code, ErrorCode::Tls);
}

struct Collected {
    lines: Vec<LogLine>,
    statuses: Vec<StreamEvent>,
}

impl Collected {
    fn last(&self) -> &StreamEvent { self.statuses.last().unwrap() }
    fn states(&self) -> Vec<StreamState> {
        self.statuses.iter().map(|s| match s { StreamEvent::Status { state, .. } => *state, _ => unreachable!() }).collect()
    }
}

fn is_final(events: &[StreamEvent]) -> bool {
    events.iter().any(|e| matches!(e, StreamEvent::Status { state, .. } if state.is_final()))
}

async fn run(c: &Ctx, pod: &str, container: &str, workload: Option<&str>, options: StreamOptions, token: &str, ms: u64, until_end: bool) -> Collected {
    let events = Arc::new(Mutex::new(Vec::<StreamEvent>::new()));
    let sink = events.clone();
    let cancel = tokio_util::sync::CancellationToken::new();
    let target = RancherTarget { url: c.rancher.url.clone(), tls: Tls { insecure: true, ca_file: None }, token: token.into() };
    let r = ContainerRef { cluster_id: "c-prod1".into(), namespace: "app-commandes".into(), pod: pod.into(), container: container.into(), workload: workload.map(str::to_string) };
    let timing = Timing { batch: Duration::from_millis(10), batch_lines: 500, retry_delays: vec![Duration::from_millis(20), Duration::from_millis(50)], idle_timeout: Duration::from_secs(5) };
    let handle = spawn(target, r, options, Arc::new(move |e| sink.lock().push(e)), timing, cancel.clone());
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    while tokio::time::Instant::now() < deadline && !(until_end && is_final(&events.lock())) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cancel.cancel();
    let _ = handle.await;
    let events = events.lock().clone();
    Collected {
        lines: events.iter().flat_map(|e| match e { StreamEvent::Lines { lines } => lines.clone(), _ => vec![] }).collect(),
        statuses: events.into_iter().filter(|e| matches!(e, StreamEvent::Status { .. })).collect(),
    }
}

fn opts(follow: bool, tail: Option<u64>, previous: bool) -> StreamOptions {
    StreamOptions { follow, tail_lines: tail, since_seconds: None, previous: previous.then_some(true) }
}

#[tokio::test]
async fn historique() {
    let c = setup().await;
    let r = run(&c, HEALTHY, "app", None, opts(false, Some(10), false), FAKE_TOKEN, 3_000, true).await;
    assert_eq!(r.lines.len(), 10);
    assert!(r.lines.iter().all(|l| l.ts.is_some() && l.text.starts_with('{')));
    assert_eq!(r.states(), vec![StreamState::Connecting, StreamState::Live, StreamState::Ended]);
    assert!(matches!(r.last(), StreamEvent::Status { message: Some(m), .. } if m == "Fin de l’historique"));
}

#[tokio::test]
async fn suivi_reconnexion_sans_doublon_ni_trou() {
    let c = setup().await;
    c.rancher.control.interval_ms.store(20, Ordering::Relaxed);
    c.rancher.control.drop_after_ms.store(150, Ordering::Relaxed);
    let r = run(&c, HEALTHY, "app", None, opts(true, Some(5), false), FAKE_TOKEN, 1_200, false).await;
    assert!(r.states().iter().filter(|s| **s == StreamState::Reconnecting).count() >= 2, "{:?}", r.states());
    let texts: Vec<&String> = r.lines.iter().map(|l| &l.text).collect();
    let unique: std::collections::HashSet<_> = texts.iter().collect();
    assert_eq!(unique.len(), texts.len());
    let keys: Vec<String> = r.lines.iter().map(|l| ts_key(l.ts.as_ref().unwrap())).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert!(r.lines.len() > 20, "{}", r.lines.len());
}

#[tokio::test]
async fn previous() {
    let c = setup().await;
    let r = run(&c, HEALTHY, "app", None, opts(true, None, true), FAKE_TOKEN, 3_000, true).await;
    assert!(matches!(r.last(), StreamEvent::Status { state: StreamState::Error, message: Some(m), .. } if m.contains("previous terminated container")));
    let r = run(&c, CRASHING, "app", None, opts(true, Some(3), true), FAKE_TOKEN, 3_000, true).await;
    assert_eq!(r.lines.len(), 3);
    assert!(matches!(r.last(), StreamEvent::Status { message: Some(m), .. } if m == "Fin des logs précédents"));
}

#[tokio::test]
async fn pod_supprime_pendant_le_suivi() {
    let c = setup().await;
    c.rancher.control.interval_ms.store(20, Ordering::Relaxed);
    c.rancher.control.drop_after_ms.store(100, Ordering::Relaxed);
    let control = c.rancher.control.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        control.deleted.lock().insert(CRASHING.into());
    });
    let r = run(&c, CRASHING, "istio-proxy", None, opts(true, Some(1), false), FAKE_TOKEN, 3_000, true).await;
    match r.last() {
        StreamEvent::Status { state, message, replacements, .. } => {
            assert_eq!(*state, StreamState::Ended);
            assert_eq!(message.as_deref(), Some(format!("Le pod {CRASHING} a été supprimé.").as_str()));
            assert_eq!(replacements.as_deref(), Some(&[HEALTHY.to_string()][..]));
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn pod_inexistant_des_l_ouverture() {
    let c = setup().await;
    let r = run(&c, "api-commandes-5c4b3a2190-zzzzz", "app", Some("api-commandes"), opts(true, Some(1), false), FAKE_TOKEN, 3_000, true).await;
    match r.last() {
        StreamEvent::Status { state, message, replacements, .. } => {
            assert_eq!(*state, StreamState::Ended);
            assert_eq!(message.as_deref(), Some("Le pod api-commandes-5c4b3a2190-zzzzz n'existe plus."));
            assert_eq!(replacements.clone().unwrap(), vec![CRASHING.to_string(), HEALTHY.to_string()]);
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn token_refuse_sans_nouvelle_tentative() {
    let c = setup().await;
    let r = run(&c, HEALTHY, "app", None, opts(true, None, false), "x", 3_000, true).await;
    assert!(matches!(r.last(), StreamEvent::Status { state: StreamState::Error, error: Some(e), .. } if e.code == ErrorCode::Unauthorized));
    assert!(!r.states().contains(&StreamState::Reconnecting));
}
