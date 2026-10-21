//! Clé d'activation : `OCEKOUR1-<kid>.<base64url(nonce ‖ contenu chiffré ‖ tag)>.<signature>`.
//! - Signature Ed25519 (clé privée chez l'administrateur, clés publiques dans l'exe).
//! - Contenu (compressé, deflate brut) chiffré AES-256-GCM par la clé d'obfuscation de l'exe :
//!   la configuration Matomo n'est pas lisible dans la clé qui circule.
//!
//! Format identique à Ocekour 1.x : les clés déjà émises restent valides.

use crate::schema::{self as z, S};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use serde_json::{Value, json};
use std::io::{Read, Write};

pub const KEY_PREFIX: &str = "OCEKOUR1";

/// Erreur lisible pour l'écran d'activation.
#[derive(Debug, Clone, PartialEq)]
pub struct LicenceError(pub String);

impl std::fmt::Display for LicenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for LicenceError {}

fn err(m: &str) -> LicenceError { LicenceError(m.into()) }

pub fn payload_schema() -> S {
    z::object(vec![
        ("v", z::literal(json!(1))),
        ("id", z::string().pattern(r"^k-[a-z0-9]{6,20}$")),
        ("label", z::string().min(1).max(100)),
        ("issuedAt", z::string().pattern(r"^\d{4}-\d{2}-\d{2}$")),
        ("mode", z::enumeration(&["none", "optional", "required"])),
        (
            "matomo",
            z::object(vec![
                ("url", z::string().url()),
                ("siteId", z::int().min(1)),
                ("insecure", z::boolean().default(json!(false))),
                ("dimensions", z::object(vec![("version", z::int().min(1).optional()), ("key", z::int().min(1).optional())]).optional()),
            ])
            .optional(),
        ),
        (
            "preset",
            z::object(vec![
                ("connections", z::array(z::record(z::any())).max(100).optional()),
                ("profiles", z::array(z::record(z::any())).max(200).optional()),
                ("services", z::record(z::any()).optional()),
            ])
            .optional(),
        ),
    ])
    .refine(|p| {
        let none = p["mode"] == json!("none");
        let has = p.get("matomo").is_some();
        (none == has).then(|| "Matomo obligatoire pour une clé avec suivi, interdit pour une clé sans suivi".to_string())
    })
}

/// Clés acceptées : clés publiques PEM par `kid`, clés d'obfuscation possibles.
#[derive(Debug, Clone, Default)]
pub struct Keyring {
    pub public_keys: Vec<(String, String)>,
    pub obfuscation_keys: Vec<String>,
}

fn aes_key(obfuscation: &str) -> anyhow::Result<Aes256Gcm> {
    let key = STANDARD.decode(obfuscation.trim())?;
    if key.len() != 32 { anyhow::bail!("Clé d'obfuscation invalide (32 octets en base64)"); }
    Ok(Aes256Gcm::new_from_slice(&key)?)
}

static KID: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"^[a-z0-9-]{1,20}$").unwrap());
static FORMAT: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"^OCEKOUR1-([a-z0-9-]{1,20})\.([A-Za-z0-9_-]+)\.([A-Za-z0-9_-]+)$").unwrap());

/// Fabrique une clé (outil d'administration).
pub fn encode(payload: &Value, kid: &str, private_key_pem: &str, obfuscation: &str) -> anyhow::Result<String> {
    if !KID.is_match(kid) { anyhow::bail!("kid invalide : {kid}"); }
    let valid = payload_schema().parse(payload).map_err(|i| anyhow::anyhow!(i[0].describe()))?;
    let mut nonce = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce);
    let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    deflate.write_all(crate::logs::json_text(&valid).as_bytes())?;
    let packed = deflate.finish()?;
    let sealed = aes_key(obfuscation)?.encrypt(Nonce::from_slice(&nonce), packed.as_ref()).map_err(|_| anyhow::anyhow!("chiffrement impossible"))?;
    let body = URL_SAFE_NO_PAD.encode([&nonce[..], &sealed].concat());
    let signed = format!("{KEY_PREFIX}-{kid}.{body}");
    let key = SigningKey::from_pkcs8_pem(private_key_pem).map_err(|e| anyhow::anyhow!("clé privée illisible : {e}"))?;
    let signature = key.sign(signed.as_bytes());
    Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes())))
}

/// Vérifie et lit une clé ; erreur au message lisible sinon. Renvoie le contenu et le `kid`.
pub fn decode(text: &str, keyring: &Keyring) -> Result<(Value, String), LicenceError> {
    // Collée depuis un mail ou une messagerie : retours à la ligne et espaces retirés.
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let m = FORMAT.captures(&compact).ok_or_else(|| err("Ce texte n'est pas une clé Ocekour (OCEKOUR1-…)."))?;
    let (kid, body, signature) = (&m[1], &m[2], &m[3]);
    let pem = keyring
        .public_keys
        .iter()
        .find(|(k, _)| k == kid)
        .map(|(_, p)| p)
        .ok_or_else(|| LicenceError(format!("Clé signée par « {kid} », inconnue de cette version d'Ocekour.")))?;
    let altered = || err("Clé invalide ou altérée (copiée en entier ?).");
    let public = VerifyingKey::from_public_key_pem(pem).map_err(|_| altered())?;
    let sig_bytes = URL_SAFE_NO_PAD.decode(signature).map_err(|_| altered())?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|_| altered())?;
    public.verify(format!("{KEY_PREFIX}-{kid}.{body}").as_bytes(), &sig).map_err(|_| altered())?;

    let raw = URL_SAFE_NO_PAD.decode(body).map_err(|_| altered())?;
    if raw.len() < 12 + 16 { return Err(altered()); }
    let mut json: Option<String> = None;
    for obfuscation in &keyring.obfuscation_keys {
        let Ok(cipher) = aes_key(obfuscation) else { continue };
        let Ok(packed) = cipher.decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]) else { continue };
        let mut out = String::new();
        if flate2::read::DeflateDecoder::new(packed.as_slice()).take(1024 * 1024).read_to_string(&mut out).is_ok() {
            json = Some(out);
            break;
        }
    }
    let json = json.ok_or_else(|| err("Clé illisible par cette version d’Ocekour."))?;
    let data: Value = serde_json::from_str(&json).map_err(|_| err("Clé illisible."))?;
    if data.is_object() && data["v"] != json!(1) {
        return Err(err("Clé créée pour une version plus récente d’Ocekour : mettez-la à jour."));
    }
    let parsed = payload_schema().parse(&data).map_err(|_| err("Clé au contenu invalide."))?;
    Ok((parsed, kid.to_string()))
}

/// Nouvelle paire Ed25519 (PEM PKCS#8 / SPKI).
pub fn generate_signing_keys() -> (String, String) {
    let mut seed = [0u8; 32];
    rand::rng().fill_bytes(&mut seed);
    let key = SigningKey::from_bytes(&seed);
    let private = key.to_pkcs8_pem(Default::default()).unwrap().to_string();
    let public = key.verifying_key().to_public_key_pem(Default::default()).unwrap();
    (private, public)
}

pub fn generate_obfuscation_key() -> String {
    let mut k = [0u8; 32];
    rand::rng().fill_bytes(&mut k);
    STANDARD.encode(k)
}

/// Identifiant de clé : `k-` + 8 caractères aléatoires.
pub fn new_licence_id() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut raw = [0u8; 8];
    rand::rng().fill_bytes(&mut raw);
    format!("k-{}", raw.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect::<String>())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn payload(overrides: Value) -> Value {
        let mut p = json!({
            "v": 1, "id": new_licence_id(), "label": "Équipe Paiement", "issuedAt": "2026-09-24", "mode": "optional",
            "matomo": { "url": "https://matomo.intra/", "siteId": 12, "insecure": false, "dimensions": { "key": 2 } },
        });
        for (k, v) in overrides.as_object().unwrap() {
            if v.is_null() { p.as_object_mut().unwrap().shift_remove(k); } else { p[k] = v.clone(); }
        }
        p
    }

    pub struct Fixture {
        pub private: String,
        pub obfuscation: String,
        pub keyring: Keyring,
    }

    pub fn fixture() -> Fixture {
        let (private, public) = generate_signing_keys();
        let obfuscation = generate_obfuscation_key();
        let keyring = Keyring { public_keys: vec![("prod1".into(), public)], obfuscation_keys: vec![obfuscation.clone()] };
        Fixture { private, obfuscation, keyring }
    }

    #[test]
    fn aller_retour_meme_sur_plusieurs_lignes() {
        let f = fixture();
        let p = payload(json!({}));
        let key = encode(&p, "prod1", &f.private, &f.obfuscation).unwrap();
        assert!(regex::Regex::new(r"^OCEKOUR1-prod1\.[\w-]+\.[\w-]+$").unwrap().is_match(&key));
        let wrapped: String = key.chars().collect::<Vec<_>>().chunks(60).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("\n  ");
        let (decoded, kid) = decode(&wrapped, &f.keyring).unwrap();
        assert_eq!(decoded, p);
        assert_eq!(kid, "prod1");
    }

    #[test]
    fn matomo_illisible_dans_la_cle() {
        let f = fixture();
        let key = encode(&payload(json!({})), "prod1", &f.private, &f.obfuscation).unwrap();
        let decoded: String = key.split('.').map(|part| String::from_utf8_lossy(&URL_SAFE_NO_PAD.decode(part).unwrap_or_default()).into_owned()).collect();
        assert!(!decoded.contains("matomo"));
        assert!(!decoded.contains("Paiement"));
    }

    #[test]
    fn refuse_alteree_autre_signataire_inconnue() {
        let f = fixture();
        let key = encode(&payload(json!({})), "prod1", &f.private, &f.obfuscation).unwrap();
        let parts: Vec<&str> = key.split('.').collect();
        let body = parts[1];
        let flipped = format!("{}{}{}", &body[..10], if &body[10..11] == "A" { "B" } else { "A" }, &body[11..]);
        assert!(decode(&format!("{}.{flipped}.{}", parts[0], parts[2]), &f.keyring).unwrap_err().0.contains("altérée"));
        let other = generate_signing_keys();
        let forged = encode(&payload(json!({ "mode": "none", "matomo": null })), "prod1", &other.0, &f.obfuscation).unwrap();
        assert!(decode(&forged, &f.keyring).unwrap_err().0.contains("altérée"));
        let unknown = encode(&payload(json!({})), "prod9", &f.private, &f.obfuscation).unwrap();
        assert!(decode(&unknown, &f.keyring).unwrap_err().0.contains("inconnue"));
        assert!(decode("bonjour", &f.keyring).unwrap_err().0.contains("pas une clé Ocekour"));
        let mut other_obf = f.keyring.clone();
        other_obf.obfuscation_keys = vec![generate_obfuscation_key()];
        assert!(decode(&key, &other_obf).unwrap_err().0.contains("illisible"));
    }

    #[test]
    fn preconfiguration_compressee_et_regles_matomo() {
        let f = fixture();
        let connections: Vec<Value> = (0..10).map(|i| json!({ "type": "kafka", "name": format!("Kafka {i}"), "brokers": [format!("kafka{i}.intra:9092")] })).collect();
        let p = payload(json!({ "preset": { "connections": connections } }));
        let key = encode(&p, "prod1", &f.private, &f.obfuscation).unwrap();
        assert_eq!(decode(&key, &f.keyring).unwrap().0["preset"], json!({ "connections": connections }));
        assert!(key.len() < 1_200, "{}", key.len());
        assert!(encode(&payload(json!({ "matomo": null })), "prod1", &f.private, &f.obfuscation).is_err());
        assert!(encode(&payload(json!({ "mode": "none" })), "prod1", &f.private, &f.obfuscation).is_err());
    }

    /// Clé de développement produite par Ocekour 1.x (`node scripts/cle.mts generer --kid dev`).
    #[test]
    fn cle_ocekour_1_acceptee() {
        let pem = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/cles-dev/dev.pem")).unwrap();
        let keyring = Keyring {
            public_keys: crate::licence::keys::DEV_PUBLIC_KEYS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            obfuscation_keys: vec![crate::licence::keys::DEV_OBFUSCATION_KEY.into()],
        };
        // Produite par Node (crypto + zlib), comme `scripts/cle.mts` d'Ocekour 1.x.
        let from_node = "OCEKOUR1-dev.F9mLmzXQlv4T1MutkH2SO0C1gvg0Sifj5408vOBBu08SHve3_38Y8WWvCfwAcDdKFLM_2SN_Bz6ak6J1XyDeWUsXgsbVJHmVMeyHvMtIPmNhkJVGv48rPOXMJbCWsN2K6g18tlXmWxAy7GN_G4s0Sv5h1O3jhc3UFW9clDq-EYtY6oLXcS2MYoRjDiRRdEXerBXztA1T3_MqrRUIHVcynN2lFluCR2A2HOKt8hPG3_8t6BBQq6p5sc3_yQTQGMKh1WfEV3PDk1Rg0VXXQUKGVr9FPvaERDEu2hlBpUcED-gKPTyt.Os6FfNIBoG3shGQYqHvLf9ghVdDJlebWACiYyCtn-C-4KtAT6R9wahpWNyGcqbVlUmPlgvCVESPKtyG1G5EvAQ";
        let (p, kid) = decode(from_node, &keyring).unwrap();
        assert_eq!(kid, "dev");
        assert_eq!(p["label"], json!("Équipe Node"));
        assert_eq!(p["preset"]["connections"][0]["brokers"], json!(["k1:9092"]));
        // La clé privée de dev correspond bien à la clé publique embarquée.
        let key = encode(&payload(json!({ "mode": "none", "matomo": null })), "dev", &pem, crate::licence::keys::DEV_OBFUSCATION_KEY).unwrap();
        assert_eq!(decode(&key, &keyring).unwrap().0["mode"], json!("none"));
    }
}
