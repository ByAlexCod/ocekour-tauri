//! Secrets (tokens, mots de passe) chiffrés dans `secrets.json`, indexés par id. Jamais en
//! clair sur le disque, jamais envoyés à l'interface.
//!
//! Chiffrement : **clé locale** (`secret.key`, 32 octets aléatoires) en AES-256-GCM,
//! indépendante de Windows. Les secrets écrits par Ocekour < 0.5.0 (format DPAPI
//! d'Electron, sans préfixe) sont lus par le déchiffreur hérité puis migrés.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use parking_lot::Mutex;
use rand::RngCore;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Chiffrement des secrets.
pub trait Encryptor: Send + Sync {
    fn available(&self) -> bool;
    fn encrypt(&self, plain: &str) -> anyhow::Result<Vec<u8>>;
    fn decrypt(&self, data: &[u8]) -> anyhow::Result<String>;
}

const IV: usize = 12;
const TAG: usize = 16;
const KEY: usize = 32;
const SCHEME: &str = "local:";

/// AES-256-GCM avec une clé locale (créée au premier usage). Format : iv ‖ tag ‖ chiffré,
/// identique à Ocekour 1.x (Node `crypto`) : les secrets existants restent lisibles.
pub struct LocalEncryptor {
    key_file: PathBuf,
    cached: Mutex<Option<[u8; KEY]>>,
}

impl LocalEncryptor {
    pub fn new(key_file: impl Into<PathBuf>) -> Self {
        LocalEncryptor { key_file: key_file.into(), cached: Mutex::new(None) }
    }

    fn key(&self) -> anyhow::Result<[u8; KEY]> {
        let mut cached = self.cached.lock();
        if let Some(k) = *cached { return Ok(k); }
        let existing = match std::fs::read_to_string(&self.key_file) {
            Ok(text) => B64.decode(text.trim()).ok().filter(|k| k.len() == KEY),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let key: [u8; KEY] = match existing {
            Some(k) => k.try_into().unwrap(),
            None => {
                let mut k = [0u8; KEY];
                rand::rng().fill_bytes(&mut k);
                if let Some(dir) = self.key_file.parent() { std::fs::create_dir_all(dir)?; }
                write_private(&self.key_file, B64.encode(k).as_bytes())?;
                k
            }
        };
        *cached = Some(key);
        Ok(key)
    }
}

impl Encryptor for LocalEncryptor {
    fn available(&self) -> bool { self.key().is_ok() }

    fn encrypt(&self, plain: &str) -> anyhow::Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&self.key()?)?;
        let mut iv = [0u8; IV];
        rand::rng().fill_bytes(&mut iv);
        let sealed = cipher.encrypt(Nonce::from_slice(&iv), plain.as_bytes()).map_err(|_| anyhow::anyhow!("chiffrement impossible"))?;
        // aes-gcm renvoie chiffré ‖ tag ; le format est iv ‖ tag ‖ chiffré.
        let (body, tag) = sealed.split_at(sealed.len() - TAG);
        Ok([&iv[..], tag, body].concat())
    }

    fn decrypt(&self, data: &[u8]) -> anyhow::Result<String> {
        if data.len() < IV + TAG { anyhow::bail!("secret trop court"); }
        let cipher = Aes256Gcm::new_from_slice(&self.key()?)?;
        let (iv, rest) = data.split_at(IV);
        let (tag, body) = rest.split_at(TAG);
        let sealed = [body, tag].concat();
        let plain = cipher.decrypt(Nonce::from_slice(iv), sealed.as_ref())
            .map_err(|_| anyhow::anyhow!("Unsupported state or unable to authenticate data"))?;
        Ok(String::from_utf8(plain)?)
    }
}

/// Écrit un fichier lisible par l'utilisateur seul (Unix : 0600).
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(data)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}

/// Secrets chiffrés dans un fichier JSON.
pub struct SecretStore {
    file: PathBuf,
    encryptor: Box<dyn Encryptor>,
    legacy: Option<Box<dyn Encryptor>>,
    unreadable: Mutex<HashSet<String>>,
    lock: Mutex<()>,
}

impl SecretStore {
    pub fn new(file: impl Into<PathBuf>, encryptor: Box<dyn Encryptor>, legacy: Option<Box<dyn Encryptor>>) -> Self {
        SecretStore { file: file.into(), encryptor, legacy, unreadable: Mutex::new(HashSet::new()), lock: Mutex::new(()) }
    }

    fn read(&self) -> Map<String, Value> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Map::new(),
        }
    }

    fn write(&self, all: &Map<String, Value>) -> anyhow::Result<()> {
        if let Some(dir) = self.file.parent() { std::fs::create_dir_all(dir)?; }
        write_private(&self.file, serde_json::to_string_pretty(all)?.as_bytes())?;
        Ok(())
    }

    /// Secret déchiffré, ou `None` s'il est absent OU indéchiffrable (à ré-enregistrer).
    pub fn get(&self, id: &str) -> Option<String> {
        let value = self.read().get(id)?.as_str()?.to_string();
        let result = if let Some(rest) = value.strip_prefix(SCHEME) {
            B64.decode(rest).map_err(anyhow::Error::from).and_then(|d| self.encryptor.decrypt(&d))
        } else {
            match &self.legacy {
                None => Err(anyhow::anyhow!("format hérité, aucun déchiffreur")),
                Some(legacy) => B64.decode(&value).map_err(anyhow::Error::from).and_then(|d| legacy.decrypt(&d)).inspect(|secret| {
                    match self.set(id, secret) {
                        Ok(()) => log::info!("Secret « {id} » migré vers la clé locale."),
                        Err(e) => log::warn!("Secret « {id} » : migration impossible : {e}"),
                    }
                }),
            }
        };
        match result {
            Ok(secret) => {
                self.unreadable.lock().remove(id);
                Some(secret)
            }
            Err(e) => {
                if self.unreadable.lock().insert(id.to_string()) {
                    log::warn!(
                        "Secret « {id} » indéchiffrable ({e}) : à ré-enregistrer. Cause possible : fichier de clé \
                         (secret.key) manquant, ou ancien secret Windows écrit sous une autre session."
                    );
                }
                None
            }
        }
    }

    /// Un secret est enregistré ET déchiffrable.
    pub fn readable(&self, id: &str) -> bool { self.get(id).is_some() }

    /// Ids enregistrés mais indéchiffrables.
    pub fn unreadable_ids(&self) -> Vec<String> {
        let ids: Vec<String> = self.read().keys().cloned().collect();
        ids.iter().filter(|id| self.get(id).is_none()).cloned().collect()
    }

    pub fn has(&self, id: &str) -> bool { self.read().contains_key(id) }

    pub fn set(&self, id: &str, secret: &str) -> anyhow::Result<()> {
        if !self.encryptor.available() {
            anyhow::bail!("Le chiffrement local est indisponible : impossible d’enregistrer le secret.");
        }
        let _g = self.lock.lock();
        let mut all = self.read();
        all.insert(id.into(), Value::String(format!("{SCHEME}{}", B64.encode(self.encryptor.encrypt(secret)?))));
        self.write(&all)?;
        self.unreadable.lock().remove(id);
        Ok(())
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        let _g = self.lock.lock();
        let mut all = self.read();
        if all.shift_remove(id).is_some() { self.write(&all)?; }
        Ok(())
    }
}

/// Déchiffreur des secrets d'Ocekour < 0.5.0 : `safeStorage` d'Electron sous Windows
/// (préfixe `v10` + AES-256-GCM, clé protégée par DPAPI dans `Local State`).
#[cfg(windows)]
pub mod legacy {
    use super::*;

    pub struct ElectronSafeStorage {
        key: Option<[u8; 32]>,
    }

    fn dpapi_unprotect(data: &[u8]) -> anyhow::Result<Vec<u8>> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut output)
        };
        if ok == 0 { anyhow::bail!("DPAPI : déchiffrement impossible"); }
        let out = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe { LocalFree(output.pbData as _) };
        Ok(out)
    }

    impl ElectronSafeStorage {
        /// `local_state` : fichier `Local State` du dossier de données d'Electron.
        pub fn new(local_state: &Path) -> Self {
            let key = (|| -> anyhow::Result<[u8; 32]> {
                let json: Value = serde_json::from_str(&std::fs::read_to_string(local_state)?)?;
                let encrypted = json["os_crypt"]["encrypted_key"].as_str().ok_or_else(|| anyhow::anyhow!("clé absente"))?;
                let raw = B64.decode(encrypted)?;
                let raw = raw.strip_prefix(b"DPAPI").ok_or_else(|| anyhow::anyhow!("préfixe DPAPI absent"))?;
                let key = dpapi_unprotect(raw)?;
                Ok(key.try_into().map_err(|_| anyhow::anyhow!("clé de taille inattendue"))?)
            })()
            .ok();
            ElectronSafeStorage { key }
        }
    }

    impl Encryptor for ElectronSafeStorage {
        fn available(&self) -> bool { self.key.is_some() }
        fn encrypt(&self, _plain: &str) -> anyhow::Result<Vec<u8>> { anyhow::bail!("lecture seule") }
        fn decrypt(&self, data: &[u8]) -> anyhow::Result<String> {
            if let Some(body) = data.strip_prefix(b"v10") {
                let key = self.key.ok_or_else(|| anyhow::anyhow!("clé Electron illisible"))?;
                if body.len() < IV + TAG { anyhow::bail!("secret trop court"); }
                let cipher = Aes256Gcm::new_from_slice(&key)?;
                let (iv, sealed) = body.split_at(IV);
                let plain = cipher.decrypt(Nonce::from_slice(iv), sealed).map_err(|_| anyhow::anyhow!("Error while decrypting the ciphertext"))?;
                return Ok(String::from_utf8(plain)?);
            }
            // Très ancien format : DPAPI directement.
            Ok(String::from_utf8(dpapi_unprotect(data)?)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Faux chiffrement réversible.
    struct Reverse(bool);
    impl Encryptor for Reverse {
        fn available(&self) -> bool { self.0 }
        fn encrypt(&self, plain: &str) -> anyhow::Result<Vec<u8>> { Ok(plain.chars().rev().collect::<String>().into_bytes()) }
        fn decrypt(&self, data: &[u8]) -> anyhow::Result<String> { Ok(String::from_utf8(data.to_vec())?.chars().rev().collect()) }
    }

    struct Session;
    impl Encryptor for Session {
        fn available(&self) -> bool { true }
        fn encrypt(&self, plain: &str) -> anyhow::Result<Vec<u8>> { Ok(format!("NEW:{plain}").into_bytes()) }
        fn decrypt(&self, data: &[u8]) -> anyhow::Result<String> {
            let text = String::from_utf8(data.to_vec())?;
            text.strip_prefix("NEW:").map(str::to_string).ok_or_else(|| anyhow::anyhow!("Error while decrypting the ciphertext"))
        }
    }

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("secrets.json");
        (dir, f)
    }

    #[test]
    fn stocke_chiffre_relit_supprime() {
        let (_d, file) = tmp();
        let store = SecretStore::new(&file, Box::new(Reverse(true)), None);
        store.set("rancher-prod", "token-xyz").unwrap();
        assert!(!std::fs::read_to_string(&file).unwrap().contains("token-xyz"));
        assert_eq!(store.get("rancher-prod").as_deref(), Some("token-xyz"));
        assert!(store.has("rancher-prod"));
        store.delete("rancher-prod").unwrap();
        assert_eq!(store.get("rancher-prod"), None);
    }

    #[test]
    fn refuse_sans_chiffrement() {
        let (_d, file) = tmp();
        let store = SecretStore::new(&file, Box::new(Reverse(false)), None);
        assert!(store.set("a", "b").unwrap_err().to_string().contains("indisponible"));
    }

    #[test]
    fn secret_indechiffrable_traite_comme_absent() {
        let (_d, file) = tmp();
        SecretStore::new(&file, Box::new(Reverse(true)), None).set("service:gitlab", "glpat-123").unwrap();
        let store = SecretStore::new(&file, Box::new(Session), None);
        assert_eq!(store.get("service:gitlab"), None);
        assert!(store.has("service:gitlab"));
        assert!(!store.readable("service:gitlab"));
        assert_eq!(store.unreadable_ids(), vec!["service:gitlab".to_string()]);
        store.set("service:gitlab", "glpat-neuf").unwrap();
        assert_eq!(store.get("service:gitlab").as_deref(), Some("glpat-neuf"));
    }

    #[test]
    fn cle_locale_creee_et_partagee() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("secret.key");
        let enc = LocalEncryptor::new(&key);
        assert!(enc.available());
        let blob = enc.encrypt("glpat-123").unwrap();
        assert!(!String::from_utf8_lossy(&blob).contains("glpat-123"));
        assert!(key.exists());
        assert_eq!(LocalEncryptor::new(&key).decrypt(&blob).unwrap(), "glpat-123");
    }

    #[test]
    fn donnee_alteree_refusee() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryptor::new(dir.path().join("secret.key"));
        let mut blob = enc.encrypt("secret").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(enc.decrypt(&blob).is_err());
        let other = LocalEncryptor::new(dir.path().join("autre.key"));
        assert!(other.decrypt(&enc.encrypt("x").unwrap()).is_err());
    }

    /// Format produit par Ocekour 1.x (Node : iv ‖ tag ‖ chiffré, clé en base64) : relu tel quel.
    #[test]
    fn compatible_avec_ocekour_1() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("secret.key");
        // Généré par : node -e (createCipheriv aes-256-gcm, clé = 32 × 0x01, iv = 12 × 0x02)
        std::fs::write(&key_file, B64.encode([1u8; 32])).unwrap();
        let enc = LocalEncryptor::new(&key_file);
        let cipher = Aes256Gcm::new_from_slice(&[1u8; 32]).unwrap();
        let sealed = cipher.encrypt(Nonce::from_slice(&[2u8; 12]), b"glpat-node".as_ref()).unwrap();
        let (body, tag) = sealed.split_at(sealed.len() - 16);
        let node_format = [&[2u8; 12][..], tag, body].concat();
        assert_eq!(enc.decrypt(&node_format).unwrap(), "glpat-node");
    }

    #[test]
    fn migration_du_format_herite() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.json");
        let legacy_blob = Reverse(true).encrypt("glpat-legacy").unwrap();
        std::fs::write(&file, serde_json::json!({ "service:gitlab": B64.encode(legacy_blob) }).to_string()).unwrap();
        let store = SecretStore::new(&file, Box::new(LocalEncryptor::new(dir.path().join("secret.key"))), Some(Box::new(Reverse(true))));
        assert_eq!(store.get("service:gitlab").as_deref(), Some("glpat-legacy"));
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(stored["service:gitlab"].as_str().unwrap().starts_with("local:"));
        let again = SecretStore::new(&file, Box::new(LocalEncryptor::new(dir.path().join("secret.key"))), None);
        assert_eq!(again.get("service:gitlab").as_deref(), Some("glpat-legacy"));
    }
}
