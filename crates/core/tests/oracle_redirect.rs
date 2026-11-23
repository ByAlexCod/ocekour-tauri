//! Oracle : le listener peut rediriger le client (dispatcher de serveur partagé, RAC / SCAN,
//! Connection Manager). Le pilote doit se reconnecter à l'adresse donnée avec la chaîne de
//! connexion fournie, au lieu d'échouer sur « redirect not implemented ».
//! Deux faux listeners : le premier redirige, le second note le CONNECT reçu puis refuse.

use oracle_rs::{Config, Connection};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// Paquet TNS (SDU classique) : longueur, somme, type, drapeaux, somme d'en-tête, données.
fn packet(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = ((8 + payload.len()) as u16).to_be_bytes().to_vec();
    p.extend_from_slice(&[0, 0, kind, 0, 0, 0]);
    p.extend_from_slice(payload);
    p
}

/// Lit un paquet entier ; renvoie son type et ses données.
async fn read_packet(stream: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).await.ok()?;
    let len = u16::from_be_bytes([header[0], header[1]]) as usize;
    let mut body = vec![0u8; len.saturating_sub(8)];
    stream.read_exact(&mut body).await.ok()?;
    Some((header[4], body))
}

/// Données du CONNECT (chaîne de connexion), y compris quand elles suivent dans un paquet DATA.
async fn read_connect(stream: &mut TcpStream) -> String {
    let (kind, body) = read_packet(stream).await.expect("CONNECT");
    assert_eq!(kind, 1, "CONNECT attendu");
    // Longueur et position des données (depuis le début du paquet), cf. ConnectMessage::build.
    let len = u16::from_be_bytes([body[16], body[17]]) as usize;
    let offset = u16::from_be_bytes([body[18], body[19]]) as usize - 8;
    if body.len() >= offset + len && len > 0 {
        return String::from_utf8_lossy(&body[offset..offset + len]).into();
    }
    let (_, data) = read_packet(stream).await.expect("DATA du CONNECT");
    String::from_utf8_lossy(&data[2..]).into()
}

async fn target_listener() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let connect = read_connect(&mut stream).await;
            s.lock().await.push(connect);
            // REFUSE (raisons 0, 0, pas de données).
            let _ = stream.write_all(&packet(4, &[0, 0, 0, 0])).await;
        }
    });
    (port, seen)
}

/// Premier listener : répond REDIRECT vers `to` ; `split` : données dans un paquet DATA à part.
async fn redirecting_listener(to: u16, split: bool) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _ = read_connect(&mut stream).await;
            let data = format!("(ADDRESS=(PROTOCOL=TCP)(HOST=127.0.0.1)(PORT={to}))\0(DESCRIPTION=(CONNECT_DATA=(SERVICE_NAME=REDIRIGE)))");
            let mut redirect = (data.len() as u16).to_be_bytes().to_vec();
            if split {
                let _ = stream.write_all(&packet(5, &redirect)).await;
                let mut body = vec![0, 0];
                body.extend_from_slice(data.as_bytes());
                let _ = stream.write_all(&packet(6, &body)).await;
            } else {
                redirect.extend_from_slice(data.as_bytes());
                let _ = stream.write_all(&packet(5, &redirect)).await;
            }
        }
    });
    port
}

#[tokio::test]
async fn redirection_du_listener_suivie() {
    for split in [false, true] {
        let (target, seen) = target_listener().await;
        let port = redirecting_listener(target, split).await;
        let config = Config::new("127.0.0.1", port, "ORIGINE", "lecteur", "secret");
        let error = match Connection::connect_with_config(config).await {
            Ok(_) => panic!("connexion inattendue"),
            Err(e) => e.to_string(),
        };
        assert!(!error.contains("redirect"), "redirection non suivie (split={split}) : {error}");
        assert!(error.to_lowercase().contains("refused"), "{error}");
        let seen = seen.lock().await;
        assert_eq!(seen.len(), 1, "le second listener a reçu le CONNECT (split={split})");
        assert!(seen[0].contains("SERVICE_NAME=REDIRIGE"), "chaîne de connexion de la redirection : {}", seen[0]);
    }
}
