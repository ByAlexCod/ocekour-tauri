//! Authentification SASL SCRAM-SHA-256 / SCRAM-SHA-512 (RFC 5802), côté client.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256, Sha512};

pub struct Scram {
    sha512: bool,
    user: String,
    password: String,
    nonce: String,
    client_first_bare: String,
    server_signature: Vec<u8>,
}

fn hmac(sha512: bool, key: &[u8], data: &[u8]) -> Vec<u8> {
    if sha512 {
        let mut m = Hmac::<Sha512>::new_from_slice(key).unwrap();
        m.update(data);
        m.finalize().into_bytes().to_vec()
    } else {
        let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
        m.update(data);
        m.finalize().into_bytes().to_vec()
    }
}

fn hash(sha512: bool, data: &[u8]) -> Vec<u8> {
    if sha512 { Sha512::digest(data).to_vec() } else { Sha256::digest(data).to_vec() }
}

fn salted_password(sha512: bool, password: &[u8], salt: &[u8], iterations: u32) -> Vec<u8> {
    if sha512 {
        let mut out = [0u8; 64];
        pbkdf2::pbkdf2_hmac::<Sha512>(password, salt, iterations, &mut out);
        out.to_vec()
    } else {
        let mut out = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut out);
        out.to_vec()
    }
}

fn escape(name: &str) -> String { name.replace('=', "=3D").replace(',', "=2C") }

impl Scram {
    pub fn new(mechanism: &str, user: &str, password: &str) -> Self {
        let mut raw = [0u8; 24];
        rand::rng().fill_bytes(&mut raw);
        Self::with_nonce(mechanism, user, password, &B64.encode(raw))
    }

    pub fn with_nonce(mechanism: &str, user: &str, password: &str, nonce: &str) -> Self {
        Scram {
            sha512: mechanism.ends_with("512"),
            user: user.into(),
            password: password.into(),
            nonce: nonce.into(),
            client_first_bare: String::new(),
            server_signature: vec![],
        }
    }

    pub fn client_first(&mut self) -> String {
        self.client_first_bare = format!("n={},r={}", escape(&self.user), self.nonce);
        format!("n,,{}", self.client_first_bare)
    }

    /// Réponse au premier message du serveur (`r=…,s=…,i=…`).
    pub fn client_final(&mut self, server_first: &str) -> Result<String, String> {
        let field = |k: &str| server_first.split(',').find_map(|p| p.strip_prefix(&format!("{k}="))).map(str::to_string);
        let nonce = field("r").ok_or("SCRAM : nonce absent")?;
        if !nonce.starts_with(&self.nonce) { return Err("SCRAM : nonce du serveur invalide".into()); }
        let salt = B64.decode(field("s").ok_or("SCRAM : sel absent")?).map_err(|_| "SCRAM : sel illisible")?;
        let iterations: u32 = field("i").and_then(|i| i.parse().ok()).ok_or("SCRAM : itérations absentes")?;
        let salted = salted_password(self.sha512, self.password.as_bytes(), &salt, iterations);
        let client_key = hmac(self.sha512, &salted, b"Client Key");
        let stored_key = hash(self.sha512, &client_key);
        let without_proof = format!("c=biws,r={nonce}");
        let auth_message = format!("{},{server_first},{without_proof}", self.client_first_bare);
        let signature = hmac(self.sha512, &stored_key, auth_message.as_bytes());
        let proof: Vec<u8> = client_key.iter().zip(signature).map(|(a, b)| a ^ b).collect();
        let server_key = hmac(self.sha512, &salted, b"Server Key");
        self.server_signature = hmac(self.sha512, &server_key, auth_message.as_bytes());
        Ok(format!("{without_proof},p={}", B64.encode(proof)))
    }

    /// Vérifie la signature finale du serveur (`v=…`).
    pub fn verify_server(&self, server_final: &str) -> Result<(), String> {
        if let Some(e) = server_final.strip_prefix("e=") { return Err(format!("SCRAM : {e}")); }
        let v = server_final.split(',').find_map(|p| p.strip_prefix("v=")).ok_or("SCRAM : signature du serveur absente")?;
        if B64.decode(v).map_err(|_| "SCRAM : signature illisible")? != self.server_signature {
            return Err("SCRAM : signature du serveur invalide".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exemple de la RFC 7677 (SCRAM-SHA-256).
    #[test]
    fn rfc7677() {
        let mut s = Scram::with_nonce("SCRAM-SHA-256", "user", "pencil", "rOprNGfwEbeRWgbNEkqO");
        assert_eq!(s.client_first(), "n,,n=user,r=rOprNGfwEbeRWgbNEkqO");
        let fin = s
            .client_final("r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096")
            .unwrap();
        assert_eq!(fin, "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ=");
        assert!(s.verify_server("v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=").is_ok());
        assert!(s.verify_server("v=AAAA").is_err());
    }
}
