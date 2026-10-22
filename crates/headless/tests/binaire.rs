//! L'exécutable sans interface, de bout en bout : clés d'activation, serveur MCP stdio (avec
//! activation par l'outil), serveur MCP HTTP.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Processus tué même si le test échoue.
struct Killed(std::process::Child);
impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bin() -> Command { Command::new(env!("CARGO_BIN_EXE_ocekour-headless")) }

fn dev_key() -> String {
    let out = bin().args(["cle", "generer", "--kid", "dev", "--label", "Tests", "--mode", "none"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn cles_generer_et_lire() {
    let key = dev_key();
    assert!(key.starts_with("OCEKOUR1-"), "{key}");
    let out = bin().args(["cle", "lire", &key]).output().unwrap();
    let payload: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!((payload["label"].as_str(), payload["mode"].as_str()), (Some("Tests"), Some("none")));
    let bad = bin().args(["cle", "generer", "--kid", "dev", "--label", "x", "--mode", "required"]).output().unwrap();
    assert!(!bad.status.success() && String::from_utf8_lossy(&bad.stderr).contains("--matomo-url"));
    let tracked = bin().args(["cle", "generer", "--kid", "dev", "--label", "x", "--mode", "required", "--matomo-url", "https://m.intra/", "--site-id", "3"]).output().unwrap();
    assert!(tracked.status.success());
    assert!(!bin().args(["cle", "lire", "OCEKOUR1-abc"]).output().unwrap().status.success());
    let usage = bin().output().unwrap();
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bin().arg("--version").output().unwrap().stdout).starts_with("ocekour-headless "));
}

#[test]
fn serveur_mcp_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = bin().arg("--mcp").arg(format!("--data-dir={}", dir.path().display())).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut ask = |m: Value| -> Value {
        writeln!(stdin, "{m}").unwrap();
        stdin.flush().unwrap();
        serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap()
    };
    let init = ask(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "essai", "version": "1" } } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "ocekour");
    let refused = ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "connexions_lister", "arguments": {} } }));
    assert_eq!(refused["result"]["isError"], true);
    assert!(refused["result"]["content"][0]["text"].as_str().unwrap().contains("licence_activer"));
    let activated = ask(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "licence_activer", "arguments": { "cle": dev_key() } } }));
    assert!(activated["result"]["content"][0]["text"].as_str().unwrap().contains("\"active\""), "{activated}");
    let listed = ask(json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "connexions_lister", "arguments": {} } }));
    assert_eq!(listed["result"]["content"][0]["text"], "[]");
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() { assert!(status.success()); break; }
        assert!(Instant::now() < deadline, "le serveur doit s'arrêter quand l'entrée se ferme");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Rien d'autre que le protocole sur la sortie standard ; le journal est dans le dossier.
    assert!(lines.next().is_none());
    assert!(std::fs::read_to_string(dir.path().join("logs/main.log")).unwrap().contains("Serveur MCP (stdio) prêt"));
    assert!(dir.path().join("secrets.json").exists());
}

#[test]
fn serveur_mcp_http() {
    let dir = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = Killed(bin().arg("--mcp-http").arg(format!("--mcp-port={port}")).arg(format!("--data-dir={}", dir.path().display())).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap());
    let mut err = BufReader::new(child.0.stderr.take().unwrap()).lines();
    let url = format!("Serveur MCP : http://127.0.0.1:{port}/mcp");
    assert!(err.by_ref().map(|l| l.unwrap()).any(|l| l == url));
    let token = err.next().unwrap().unwrap().strip_prefix("Jeton : ").unwrap().to_string();
    assert!(token.len() >= 32);
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string();
    let mut tcp = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(tcp, "POST /mcp HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    tcp.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
    let mut tcp = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(tcp, "POST /mcp HTTP/1.1\r\nhost: localhost:{port}\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    tcp.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200") && response.contains("\"result\":{}"), "{response}");
    drop(child);
}
