//! `sonde.yaml` : adresse d'écoute, certificat, jeton (haché), sources exposées, limites.
//! Création (`sonde init`), renouvellement du jeton, lecture validée (erreurs avec ligne).

use crate::schema::{self as z, S};
use crate::streams::encoding::Encoding;
use crate::streams::file::FileTarget;
use crate::streams::pattern::Pattern;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 47120;
pub const CONFIG_FILE: &str = "sonde.yaml";
pub const CERT_FILE: &str = "sonde.crt";
pub const KEY_FILE: &str = "sonde.key";
pub const STATUS_FILE: &str = "sonde-statut.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceConfig {
    pub id: String,
    pub nom: Option<String>,
    pub fichier: Option<String>,
    pub motif: Option<String>,
    pub encodage: Option<String>,
    /// Motif : au plus N fichiers listés (les plus récents).
    pub garder: Option<usize>,
}

impl SourceConfig {
    pub fn name(&self) -> String { self.nom.clone().unwrap_or_else(|| self.id.clone()) }

    pub fn target(&self) -> FileTarget {
        match (&self.fichier, &self.motif) {
            (_, Some(m)) => FileTarget::Pattern(Pattern::parse(m).expect("motif validé")),
            (Some(f), None) => FileTarget::File(PathBuf::from(f)),
            (None, None) => unreachable!("source validée"),
        }
    }

    pub fn encoding(&self) -> Option<Encoding> { self.encodage.as_deref().and_then(Encoding::parse) }

    pub fn is_pattern(&self) -> bool { self.motif.is_some() }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tls {
    pub certificat: Option<String>,
    pub cle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Limits {
    pub flux_simultanes: usize,
    pub flux_par_poste: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SondeConfig {
    pub ecoute: String,
    pub nom: Option<String>,
    pub tls: Option<Tls>,
    /// `sha256:<hex>` du jeton.
    pub jeton: String,
    pub sources: Vec<SourceConfig>,
    pub limites: Limits,
}

/// Sonde chargée : configuration et chemins résolus (relatifs au dossier de sonde.yaml).
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub config: SondeConfig,
    pub dir: PathBuf,
    pub addr: SocketAddr,
    /// Certificat et clé (PEM), absents seulement en écoute locale sans TLS.
    pub tls: Option<(PathBuf, PathBuf)>,
}

impl Loaded {
    pub fn name(&self) -> String {
        self.config.nom.clone().unwrap_or_else(hostname)
    }
    pub fn source(&self, id: &str) -> Option<&SourceConfig> { self.config.sources.iter().find(|s| s.id == id) }
}

fn schema() -> S {
    z::object(vec![
        ("ecoute", z::string().default(json!(format!("0.0.0.0:{DEFAULT_PORT}")))),
        ("nom", z::string().min(1).optional()),
        ("tls", z::object(vec![("certificat", z::string().min(1).optional()), ("cle", z::string().min(1).optional())]).optional()),
        ("jeton", z::string().pattern("^sha256:[0-9a-f]{64}$").describe("Empreinte du jeton (sonde jeton renouveler)")),
        (
            "sources",
            z::array(z::object(vec![
                ("id", z::string().pattern("^[a-z0-9][a-z0-9-]{0,63}$")),
                ("nom", z::string().min(1).optional()),
                ("fichier", z::string().min(1).optional()),
                ("motif", z::string().min(1).optional()),
                ("encodage", z::enumeration(&["utf-8", "utf-16", "utf-16le", "utf-16be", "latin-1", "windows-1252"]).optional()),
                ("garder", z::int().min(1).max(1_000).optional()),
            ]))
            .default(json!([])),
        ),
        (
            "limites",
            z::object(vec![
                ("flux_simultanes", z::int().min(1).max(10_000).default(json!(50))),
                ("flux_par_poste", z::int().min(1).max(10_000).default(json!(10))),
            ])
            .default(json!({})),
        ),
    ])
}

/// Erreur de sonde.yaml, avec sa ligne si possible.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub message: String,
    pub line: Option<usize>,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(l) => write!(f, "sonde.yaml, ligne {l} : {}", self.message),
            None => write!(f, "sonde.yaml : {}", self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

fn resolve(dir: &Path, p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() { path } else { dir.join(path) }
}

/// Lit et valide sonde.yaml.
pub fn load(path: &Path) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError { message: format!("{} : {e}", path.display()), line: None })?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let dir = if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir };
    parse(&text, &dir)
}

pub fn parse(text: &str, dir: &Path) -> Result<Loaded, ConfigError> {
    let doc = crate::yaml::parse(text).map_err(|e| ConfigError { message: e.message, line: Some(e.line) })?;
    let mut raw = doc.to_json();
    if raw.is_null() { raw = json!({}); }
    let at = |path: Vec<crate::yaml::P>| doc.position(&path).map(|p| p.0);
    let value = schema().parse(&raw).map_err(|issues| {
        let i = &issues[0];
        ConfigError { message: i.describe(), line: at(crate::config::issue_path(i)) }
    })?;
    let config: SondeConfig = serde_json::from_value(value).map_err(|e| ConfigError { message: e.to_string(), line: None })?;
    use crate::yaml::P;
    let addr: SocketAddr = config.ecoute.parse().map_err(|_| ConfigError {
        message: format!("ecoute : adresse:port attendu (ex. 0.0.0.0:{DEFAULT_PORT}), pas « {} »", config.ecoute),
        line: at(vec![P::K("ecoute".into())]),
    })?;
    let mut seen = std::collections::HashSet::new();
    for (i, s) in config.sources.iter().enumerate() {
        let line = at(vec![P::K("sources".into()), P::I(i)]);
        let fail = |message: String| ConfigError { message, line };
        if !seen.insert(s.id.clone()) { return Err(fail(format!("source « {} » en double", s.id))); }
        match (&s.fichier, &s.motif) {
            (Some(_), Some(_)) | (None, None) => return Err(fail(format!("source « {} » : « fichier » ou « motif » (un seul des deux)", s.id))),
            (Some(f), None) => {
                if !Path::new(f).is_absolute() { return Err(fail(format!("source « {} » : chemin absolu attendu", s.id))); }
                if crate::streams::pattern::is_pattern(f) { return Err(fail(format!("source « {} » : jokers dans « fichier » (utilisez « motif »)", s.id))); }
            }
            (None, Some(m)) => { Pattern::parse(m).map_err(|e| fail(format!("source « {} » : {}", s.id, e.message)))?; }
        }
    }
    let tls = match &config.tls {
        Some(Tls { certificat: Some(c), cle: Some(k) }) => Some((resolve(dir, c), resolve(dir, k))),
        Some(Tls { certificat: None, cle: None }) | None => None,
        Some(_) => return Err(ConfigError { message: "tls : « certificat » et « cle » vont ensemble".into(), line: at(vec![P::K("tls".into())]) }),
    };
    if tls.is_none() && !addr.ip().is_loopback() {
        return Err(ConfigError {
            message: format!("TLS obligatoire pour écouter sur {} (hors 127.0.0.1) : renseignez tls.certificat et tls.cle (sonde init les crée)", addr.ip()),
            line: at(vec![P::K("ecoute".into())]),
        });
    }
    Ok(Loaded { config, dir: dir.to_path_buf(), addr, tls })
}

// --- Jeton --------------------------------------------------------------------------------------

pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("osk_{}", hex::encode(bytes))
}

pub fn token_hash(token: &str) -> String { format!("sha256:{}", hex::encode(Sha256::digest(token.trim().as_bytes()))) }

/// Comparaison en temps constant des empreintes.
pub fn token_matches(given: &str, hash: &str) -> bool {
    let computed = token_hash(given);
    let (a, b) = (computed.as_bytes(), hash.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Nouveau jeton : son empreinte remplace l'ancienne dans sonde.yaml (le reste du fichier,
/// commentaires compris, est conservé). Renvoie le jeton en clair (affiché une seule fois).
pub fn renew_token(path: &Path) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(path)?;
    let token = new_token();
    let mut editor = crate::yaml::Editor::new(&text);
    editor.set(&[crate::yaml::P::K("jeton".into())], &json!(token_hash(&token))).map_err(|e| anyhow::anyhow!("sonde.yaml : {e:?}"))?;
    let next = editor.text.clone();
    let dir = path.parent().unwrap_or(Path::new("."));
    parse(&next, dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(path, next)?;
    Ok(token)
}

// --- Certificat ---------------------------------------------------------------------------------

/// Empreinte SHA-256 d'un certificat (DER), au format `SHA256:AB:CD:…`.
pub fn fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    format!("SHA256:{}", digest.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}

/// Normalise une empreinte saisie (casse, séparateurs, préfixe).
pub fn normalize_fingerprint(text: &str) -> Option<String> {
    let hex: String = text.trim().trim_start_matches("SHA256:").trim_start_matches("sha256:").chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 64 { return None; }
    let bytes = hex::decode(&hex).ok()?;
    Some(format!("SHA256:{}", bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")))
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok().filter(|h| !h.is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()))
        .unwrap_or_else(|| "sonde".into())
}

/// Certificat auto-signé (10 ans) pour le nom de la machine, localhost et les adresses données.
pub fn generate_certificate(names: &[String]) -> anyhow::Result<(String, String, String)> {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())?;
    params.distinguished_name.push(rcgen::DnType::CommonName, format!("Ocekour Sonde {}", names.first().cloned().unwrap_or_default()));
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    let now = chrono::Utc::now();
    use chrono::Datelike;
    params.not_after = rcgen::date_time_ymd(now.year() + 10, now.month() as u8, now.day().min(28) as u8);
    let mut sans = vec![];
    for n in names.iter().chain(["localhost".to_string(), "127.0.0.1".to_string()].iter()) {
        match n.parse::<std::net::IpAddr>() {
            Ok(ip) => sans.push(rcgen::SanType::IpAddress(ip)),
            Err(_) => if let Ok(dns) = n.clone().try_into() { sans.push(rcgen::SanType::DnsName(dns)) },
        }
    }
    sans.dedup();
    params.subject_alt_names = sans;
    let key = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    Ok((cert.pem(), key.serialize_pem(), fingerprint(cert.der())))
}

/// Ce qu'il faut transmettre aux utilisateurs après `sonde init`.
#[derive(Debug, Clone)]
pub struct Init {
    pub config: PathBuf,
    pub url: String,
    pub fingerprint: String,
    pub token: String,
}

const TEMPLATE: &str = r#"# Sonde Ocekour : expose en lecture seule les sources de logs déclarées ci-dessous.
# Les postes (interface Ocekour) se connectent à cette machine : seul ce port est à ouvrir.
ecoute: __ECOUTE__
nom: __NOM__
tls:
  certificat: sonde.crt
  cle: sonde.key
# Empreinte du jeton d'accès (jamais le jeton lui-même). Nouveau jeton :
#   ocekour-headless sonde jeton renouveler
jeton: __JETON__
# Sources exposées. id : [a-z0-9-], stable (liens, favoris). Un « fichier » OU un « motif »
# (jokers dans le nom seulement : /var/log/app/app-*.log). Encodages : utf-8 (défaut),
# utf-16, latin-1, windows-1252.
sources: []
#  - id: api-commandes
#    nom: API commandes
#    fichier: /var/log/commandes/api.log
#  - id: batch-nuit
#    motif: /var/log/batch/batch-*.log
#    garder: 30
limites:
  flux_simultanes: 50   # au total, tous postes confondus
  flux_par_poste: 10    # par poste abonné
"#;

/// Crée sonde.yaml, le certificat et le jeton dans `dir` (refuse d'écraser une sonde).
pub fn init(dir: &Path, ecoute: Option<&str>, nom: Option<&str>) -> anyhow::Result<Init> {
    std::fs::create_dir_all(dir)?;
    let config = dir.join(CONFIG_FILE);
    if config.exists() { anyhow::bail!("{} existe déjà (sonde jeton renouveler pour un nouveau jeton).", config.display()); }
    let ecoute = ecoute.unwrap_or("").trim();
    let ecoute = if ecoute.is_empty() { format!("0.0.0.0:{DEFAULT_PORT}") } else { ecoute.to_string() };
    let addr: SocketAddr = ecoute.parse().map_err(|_| anyhow::anyhow!("--ecoute : adresse:port attendu, pas « {ecoute} »"))?;
    let name = nom.map(str::to_string).unwrap_or_else(hostname);
    let mut names = vec![hostname()];
    if !addr.ip().is_unspecified() { names.push(addr.ip().to_string()); }
    let (cert, key, fp) = generate_certificate(&names)?;
    std::fs::write(dir.join(CERT_FILE), cert)?;
    write_private(&dir.join(KEY_FILE), &key)?;
    let token = new_token();
    let text = TEMPLATE
        .replace("__ECOUTE__", &ecoute)
        .replace("__NOM__", &crate::yaml::scalar(&json!(name)))
        .replace("__JETON__", &token_hash(&token));
    parse(&text, dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(&config, text)?;
    let host = if addr.ip().is_unspecified() { hostname() } else { addr.ip().to_string() };
    Ok(Init { config, url: format!("https://{host}:{}", addr.port()), fingerprint: fp, token })
}

/// Clé privée lisible par son seul propriétaire (Unix).
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_lecture_et_jeton() {
        let dir = tempfile::tempdir().unwrap();
        let init = init(dir.path(), Some("0.0.0.0:47999"), Some("srv-recette")).unwrap();
        assert!(init.token.starts_with("osk_") && init.token.len() == 68);
        assert!(init.url.ends_with(":47999") && init.fingerprint.starts_with("SHA256:"));
        assert!(super::init(dir.path(), None, None).is_err(), "pas d'écrasement");
        let loaded = load(&init.config).unwrap();
        assert_eq!(loaded.name(), "srv-recette");
        assert!(loaded.config.sources.is_empty());
        assert_eq!(loaded.config.limites, Limits { flux_simultanes: 50, flux_par_poste: 10 });
        assert!(token_matches(&init.token, &loaded.config.jeton));
        assert!(!token_matches("osk_autre", &loaded.config.jeton));
        let text = std::fs::read_to_string(&init.config).unwrap();
        assert!(!text.contains(&init.token), "jamais le jeton en clair");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.path().join(KEY_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Empreinte du certificat écrit.
        let pem = std::fs::read(dir.path().join(CERT_FILE)).unwrap();
        let der = rustls_pemfile::certs(&mut pem.as_slice()).next().unwrap().unwrap();
        assert_eq!(fingerprint(&der), init.fingerprint);

        // Renouvellement : l'ancien jeton ne correspond plus, commentaires conservés.
        let token = renew_token(&init.config).unwrap();
        let loaded = load(&init.config).unwrap();
        assert!(token_matches(&token, &loaded.config.jeton) && !token_matches(&init.token, &loaded.config.jeton));
        assert!(std::fs::read_to_string(&init.config).unwrap().contains("# Sources exposées"));
    }

    #[test]
    fn validation() {
        let dir = Path::new("/srv/sonde");
        let jeton = token_hash("x");
        let ok = |extra: &str| parse(&format!("ecoute: 127.0.0.1:1\njeton: {jeton}\n{extra}"), dir);
        assert!(ok("").is_ok());
        let e = ok("sources:\n  - id: a\n    fichier: /a.log\n  - id: a\n    fichier: /b.log\n").unwrap_err();
        assert_eq!((e.message.contains("double"), e.line), (true, Some(6)));
        assert!(ok("sources:\n  - id: a\n").unwrap_err().message.contains("un seul des deux"));
        assert!(ok("sources:\n  - id: a\n    fichier: relatif.log\n").unwrap_err().message.contains("absolu"));
        assert!(ok("sources:\n  - id: a\n    fichier: /x/*.log\n").unwrap_err().message.contains("motif"));
        assert!(ok("sources:\n  - id: a\n    motif: /x/**/a.log\n").is_err());
        assert!(ok("sources:\n  - id: A b\n    fichier: /a.log\n").is_err());
        assert!(ok("sources:\n  - id: a\n    fichier: /a.log\n    encodage: ebcdic\n").is_err());
        let e = parse(&format!("jeton: {jeton}\n"), dir).unwrap_err();
        assert!(e.message.contains("TLS obligatoire"), "{e}");
        assert!(parse("ecoute: 127.0.0.1:1\njeton: osk_clair\n", dir).unwrap_err().message.contains("jeton"));
        let tls = parse(&format!("jeton: {jeton}\ntls:\n  certificat: c.crt\n  cle: /k/c.key\n"), dir).unwrap();
        assert_eq!(tls.tls, Some((PathBuf::from("/srv/sonde/c.crt"), PathBuf::from("/k/c.key"))));
        assert!(normalize_fingerprint("sha256:ab cd").is_none());
        let fp = format!("SHA256:{}", vec!["AB"; 32].join(":"));
        assert_eq!(normalize_fingerprint(&fp.to_lowercase()), Some(fp.clone()));
        assert_eq!(normalize_fingerprint(&"ab".repeat(32)), Some(fp));
    }
}
