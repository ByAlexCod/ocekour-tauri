//! Réseau commun : TLS par connexion (autorités publiques + magasin du système — une CA
//! d'entreprise déployée par GPO fonctionne sans rien configurer — + fichier choisi, ou
//! vérification désactivée), clients HTTP, et erreurs réseau traduites en messages actionnables.

use crate::config::Tls;
use crate::error::{ErrorCode, SourceError};
use parking_lot::Mutex;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Fournisseur cryptographique (ring) : explicite, plusieurs peuvent être compilés.
pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    static P: once_cell::sync::Lazy<Arc<rustls::crypto::CryptoProvider>> =
        once_cell::sync::Lazy::new(|| Arc::new(rustls::crypto::ring::default_provider()));
    P.clone()
}

/// Installe ring comme fournisseur par défaut (bibliothèques qui l'exigent).
pub fn install_default_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Certificats d'un .cer/.crt/.pem (PEM, bundle PEM ou DER binaire).
pub fn read_ca_certs(path: &str) -> Result<Vec<CertificateDer<'static>>, SourceError> {
    let raw = std::fs::read(path).map_err(|_| SourceError::new(ErrorCode::Tls, format!("Certificat introuvable : {path}")))?;
    let pems: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut raw.as_slice()).filter_map(Result::ok).collect();
    if !pems.is_empty() { return Ok(pems); }
    // DER : un certificat X.509 lisible.
    if x509_parser::parse_x509_certificate(&raw).is_ok() {
        return Ok(vec![CertificateDer::from(raw)]);
    }
    Err(SourceError::new(ErrorCode::Tls, format!("{path} n'est pas un certificat lisible (PEM ou DER attendu).")))
}

/// Description d'un fichier de certificat (écran de connexion).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaFileInfo {
    pub path: String,
    pub subject: String,
    pub valid_to: String,
    pub expired: bool,
    pub count: usize,
}

pub fn ca_file_info(path: &str) -> Result<CaFileInfo, SourceError> {
    let certs = read_ca_certs(path)?;
    let (_, cert) = x509_parser::parse_x509_certificate(&certs[0])
        .map_err(|_| SourceError::new(ErrorCode::Tls, format!("{path} n'est pas un certificat lisible (PEM ou DER attendu).")))?;
    let subject = cert
        .subject()
        .iter_attributes()
        .filter_map(|a| {
            let name = x509_parser::objects::oid2abbrev(a.attr_type(), x509_parser::objects::oid_registry()).ok()?;
            Some(format!("{name}={}", a.as_str().ok()?))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let not_after = cert.validity().not_after.timestamp();
    let valid_to = chrono::DateTime::from_timestamp(not_after, 0).unwrap_or_default();
    Ok(CaFileInfo {
        path: path.into(),
        subject,
        valid_to: valid_to.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        expired: valid_to < chrono::Utc::now(),
        count: certs.len(),
    })
}

/// Autorités du système (magasin Windows), lues une fois.
fn base_roots() -> &'static [CertificateDer<'static>] {
    static ROOTS: once_cell::sync::Lazy<Vec<CertificateDer<'static>>> =
        once_cell::sync::Lazy::new(|| rustls_native_certs::load_native_certs().certs);
    &ROOTS
}

fn root_store(tls: &Tls) -> Result<RootCertStore, SourceError> {
    let mut store = RootCertStore::empty();
    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert in base_roots() {
        let _ = store.add(cert.clone());
    }
    if let Some(file) = &tls.ca_file {
        for cert in read_ca_certs(file)? {
            store.add(cert).map_err(|e| SourceError::new(ErrorCode::Tls, format!("{file} : certificat refusé ({e}).")))?;
        }
    }
    Ok(store)
}

/// Vérificateur qui accepte tout (« Ignorer la vérification SSL »).
#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Configuration rustls d'une connexion.
pub fn rustls_config(tls: &Tls) -> Result<ClientConfig, SourceError> {
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| SourceError::new(ErrorCode::Tls, e.to_string()))?;
    Ok(if tls.insecure {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider()))).with_no_client_auth()
    } else {
        builder.with_root_certificates(root_store(tls)?).with_no_client_auth()
    })
}

/// Client HTTP(S) pour ces réglages TLS (mis en cache). Aucun proxy (comme Ocekour 1.x).
pub fn http_client(tls: &Tls) -> Result<reqwest::Client, SourceError> {
    static CACHE: once_cell::sync::Lazy<Mutex<HashMap<(bool, Option<String>), reqwest::Client>>> =
        once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));
    let key = (tls.insecure, if tls.insecure { None } else { tls.ca_file.clone() });
    if let Some(c) = CACHE.lock().get(&key) { return Ok(c.clone()); }
    let mut config = rustls_config(tls)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(config)
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .pool_idle_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| SourceError::new(ErrorCode::Network, e.to_string()))?;
    CACHE.lock().insert(key, client.clone());
    Ok(client)
}

/// Hôte (et port) d'une URL, pour les messages.
pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| match u.port() { Some(p) => format!("{h}:{p}"), None => h.to_string() }))
        .unwrap_or_else(|| url.to_string())
}

/// Code « à la Node » d'une erreur de certificat (message de l'interface).
fn tls_code(e: &rustls::Error) -> Option<&'static str> {
    use rustls::CertificateError as C;
    match e {
        rustls::Error::InvalidCertificate(c) => Some(match c {
            C::UnknownIssuer => "UNABLE_TO_VERIFY_LEAF_SIGNATURE",
            C::Expired | C::ExpiredContext { .. } => "CERT_HAS_EXPIRED",
            C::NotValidForName | C::NotValidForNameContext { .. } => "ERR_TLS_CERT_ALTNAME_INVALID",
            C::BadSignature => "CERT_SIGNATURE_FAILURE",
            _ => "CERT_INVALID",
        }),
        _ => None,
    }
}

/// Parcourt les causes d'une erreur.
fn chain<'a>(e: &'a (dyn std::error::Error + 'static)) -> impl Iterator<Item = &'a (dyn std::error::Error + 'static)> {
    std::iter::successors(Some(e), |e| e.source())
}

/// Erreur réseau (connexion, TLS, DNS…) → message actionnable.
pub fn classify(e: &(dyn std::error::Error + 'static), url: &str) -> SourceError {
    let host = host_of(url);
    for cause in chain(e) {
        if let Some(se) = cause.downcast_ref::<SourceError>() { return se.clone(); }
        if let Some(t) = cause.downcast_ref::<rustls::Error>() {
            let code = tls_code(t).unwrap_or("TLS");
            return SourceError::new(
                ErrorCode::Tls,
                format!("Certificat de {host} non reconnu ({code}). Fournissez le certificat d'autorité (.cer) ou cochez « Ignorer la vérification SSL »."),
            );
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            if let Some(inner) = io.get_ref() {
                if let Some(t) = inner.downcast_ref::<rustls::Error>() {
                    let code = tls_code(t).unwrap_or("TLS");
                    return SourceError::new(
                        ErrorCode::Tls,
                        format!("Certificat de {host} non reconnu ({code}). Fournissez le certificat d'autorité (.cer) ou cochez « Ignorer la vérification SSL »."),
                    );
                }
            }
            use std::io::ErrorKind as K;
            let code = match io.kind() {
                K::ConnectionRefused => Some("ECONNREFUSED"),
                K::ConnectionReset | K::ConnectionAborted => Some("ECONNRESET"),
                K::HostUnreachable | K::NetworkUnreachable => Some("EHOSTUNREACH"),
                K::TimedOut => return SourceError::new(ErrorCode::Timeout, format!("Pas de réponse de {host} (délai dépassé).")),
                _ => None,
            };
            if let Some(code) = code {
                return SourceError::new(ErrorCode::Network, format!("Connexion à {host} impossible ({code})."));
            }
        }
        let text = cause.to_string();
        if text.contains("dns error") || text.contains("failed to lookup address") || text.contains("Name or service not known") {
            return SourceError::new(ErrorCode::Network, format!("Serveur {host} introuvable (DNS). Vérifiez l'URL."));
        }
    }
    let deepest = chain(e).last().map(|c| c.to_string()).unwrap_or_default();
    SourceError::new(ErrorCode::Network, if deepest.is_empty() { e.to_string() } else { deepest })
}

/// Erreur reqwest → erreur typée.
pub fn from_reqwest(e: &reqwest::Error, url: &str) -> SourceError {
    if e.is_timeout() {
        return SourceError::new(ErrorCode::Timeout, format!("Pas de réponse de {} (délai dépassé).", host_of(url)));
    }
    classify(e, url)
}

#[cfg(test)]
pub mod testing {
    //! Serveur HTTPS de test (certificat auto-signé).


    pub struct TestCert {
        pub cert_pem: String,
        pub key_pem: String,
        pub cert_der: Vec<u8>,
    }

    pub fn self_signed(name: &str) -> TestCert {
        let mut params = rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        params.distinguished_name.push(rcgen::DnType::CommonName, name);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.subject_alt_names = vec![
            rcgen::SanType::DnsName("localhost".try_into().unwrap()),
            rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap()),
        ];
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        TestCert { cert_pem: cert.pem(), key_pem: key.serialize_pem(), cert_der: cert.der().to_vec() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lit_pem_bundle_et_der() {
        let dir = tempfile::tempdir().unwrap();
        let a = testing::self_signed("fake-rancher");
        let b = testing::self_signed("autre");
        let pem = dir.path().join("ca.pem");
        std::fs::write(&pem, format!("{}{}", a.cert_pem, b.cert_pem)).unwrap();
        let der = dir.path().join("ca.cer");
        std::fs::write(&der, &a.cert_der).unwrap();
        assert_eq!(read_ca_certs(pem.to_str().unwrap()).unwrap().len(), 2);
        let info = ca_file_info(der.to_str().unwrap()).unwrap();
        assert!(info.subject.contains("fake-rancher"), "{}", info.subject);
        assert!(!info.expired);
        assert_eq!(info.count, 1);
        let bad = dir.path().join("bad.cer");
        std::fs::write(&bad, "nope").unwrap();
        assert!(ca_file_info(bad.to_str().unwrap()).unwrap_err().message.contains("pas un certificat"));
        assert!(ca_file_info("/inexistant.pem").unwrap_err().message.contains("introuvable"));
    }

    #[test]
    fn configs() {
        assert!(rustls_config(&Tls { insecure: true, ca_file: None }).is_ok());
        assert!(rustls_config(&Tls::default()).is_ok());
        assert!(http_client(&Tls::default()).is_ok());
    }
}
