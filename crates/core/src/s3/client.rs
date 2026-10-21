//! Client S3 d'entreprise (jamais AWS) : URL fournie, path-style ou virtual-host, signature
//! AWS SigV4, TLS de la connexion.

use crate::config::S3Connection;
use crate::error::{ErrorCode, SourceError};
use crate::net;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

/// Caractères non réservés (RFC 3986) : seuls laissés tels quels.
const STRICT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    if keep_slash {
        s.split('/').map(|p| utf8_percent_encode(p, STRICT).to_string()).collect::<Vec<_>>().join("/")
    } else {
        utf8_percent_encode(s, STRICT).to_string()
    }
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

pub fn sha256_hex(data: &[u8]) -> String { hex::encode(Sha256::digest(data)) }

/// Erreur S3 « brute » : statut HTTP, code S3 (`NoSuchKey`…), message.
#[derive(Debug, Clone)]
pub struct S3Error {
    pub status: Option<u16>,
    pub code: String,
    pub message: String,
    /// Erreur réseau déjà traduite.
    pub source: Option<SourceError>,
}

impl std::fmt::Display for S3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{} {}", self.code, self.message) }
}
impl std::error::Error for S3Error {}

/// Erreur S3 → message actionnable. `what` : bucket ou clé concernés.
pub fn to_source_error(e: &S3Error, what: &str) -> SourceError {
    if let Some(s) = &e.source { return s.clone(); }
    let w = if what.is_empty() { String::new() } else { format!(" ({what})") };
    match e.code.as_str() {
        "InvalidAccessKeyId" | "SignatureDoesNotMatch" | "InvalidToken" | "ExpiredToken" => {
            return SourceError::new(ErrorCode::Unauthorized, "Clé d'accès ou clé secrète refusée par le S3.");
        }
        "AccessDenied" => return SourceError::new(ErrorCode::Forbidden, format!("Accès refusé par le S3{w} : droits insuffisants pour cette clé.")),
        "NoSuchBucket" => return SourceError::new(ErrorCode::Http, format!("Bucket {what} introuvable.").replace("  ", " ")),
        "NoSuchKey" | "NotFound" => {
            return SourceError::new(ErrorCode::Http, format!("{} introuvable (supprimé entre-temps ?).", if what.is_empty() { "Objet" } else { what }));
        }
        "RequestTimeout" | "TimeoutError" => return SourceError::new(ErrorCode::Timeout, "Le S3 ne répond pas (délai dépassé)."),
        _ => {}
    }
    match e.status {
        Some(401) => SourceError::new(ErrorCode::Unauthorized, "Authentification refusée par le S3."),
        Some(403) => SourceError::new(ErrorCode::Forbidden, format!("Accès refusé par le S3{w}.")),
        Some(status) if e.code == "UnknownError" => {
            SourceError::new(ErrorCode::Http, format!("Réponse inattendue (HTTP {status}) : l'URL est-elle bien celle du S3 ?"))
        }
        Some(status) => SourceError::new(ErrorCode::Http, format!("{} (HTTP {status})", if e.message.is_empty() { &e.code } else { &e.message })),
        None => SourceError::new(ErrorCode::Network, e.message.clone()),
    }
}

pub type Result<T> = std::result::Result<T, S3Error>;

pub struct S3Client {
    endpoint: url::Url,
    region: String,
    path_style: bool,
    access_key: String,
    secret_key: String,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct XmlError {
    code: String,
    message: String,
}

impl S3Client {
    pub fn new(connection: &S3Connection, secret: &str) -> std::result::Result<Self, SourceError> {
        let endpoint = url::Url::parse(&connection.url).map_err(|_| SourceError::invalid(format!("URL invalide : {}", connection.url)))?;
        Ok(S3Client {
            endpoint,
            region: connection.region.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| "us-east-1".into()),
            path_style: connection.path_style,
            access_key: connection.access_key_id.clone(),
            secret_key: secret.to_string(),
            http: net::http_client(&connection.tls)?,
        })
    }

    /// URL d'une ressource (bucket et clé facultatifs).
    fn url(&self, bucket: Option<&str>, key: Option<&str>, query: &[(&str, String)]) -> url::Url {
        let mut u = self.endpoint.clone();
        let base_path = u.path().trim_end_matches('/').to_string();
        let mut path = base_path;
        match (bucket, self.path_style) {
            (Some(b), true) => path.push_str(&format!("/{}", uri_encode(b, false))),
            (Some(b), false) => {
                let host = format!("{b}.{}", self.endpoint.host_str().unwrap_or_default());
                let _ = u.set_host(Some(&host));
            }
            (None, _) => {}
        }
        if let Some(k) = key { path.push_str(&format!("/{}", uri_encode(k, true))); }
        if path.is_empty() { path.push('/'); }
        // Chemin déjà encodé : on évite un second encodage.
        u.set_path("");
        let mut s = u.to_string();
        if s.ends_with('/') { s.pop(); }
        s.push_str(&path);
        if !query.is_empty() {
            let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (uri_encode(k, false), uri_encode(v, false))).collect();
            q.sort();
            s.push('?');
            s.push_str(&q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
        }
        url::Url::parse(&s).unwrap()
    }

    /// Signature SigV4 : renvoie les en-têtes à ajouter.
    fn sign(&self, method: &str, url: &url::Url, extra: &[(String, String)], payload_hash: &str) -> Vec<(String, String)> {
        self.sign_at(method, url, extra, payload_hash, chrono::Utc::now())
    }

    fn sign_at(&self, method: &str, url: &url::Url, extra: &[(String, String)], payload_hash: &str, now: chrono::DateTime<chrono::Utc>) -> Vec<(String, String)> {
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let host = match url.port() {
            Some(p) => format!("{}:{p}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_string(),
        };
        let mut headers: Vec<(String, String)> = vec![
            ("host".into(), host),
            ("x-amz-content-sha256".into(), payload_hash.into()),
            ("x-amz-date".into(), amz_date.clone()),
        ];
        for (k, v) in extra {
            let k = k.to_lowercase();
            if k.starts_with("x-amz-") || k == "content-type" || k == "range" || k == "content-md5" {
                headers.push((k, v.trim().to_string()));
            }
        }
        headers.sort();
        let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
        let signed: String = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
        let canonical_query = url.query().unwrap_or("").to_string();
        let canonical = format!("{method}\n{}\n{canonical_query}\n{canonical_headers}\n{signed}\n{payload_hash}", url.path());
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(canonical.as_bytes()));
        let k_date = hmac(format!("AWS4{}", self.secret_key).as_bytes(), &date);
        let k_region = hmac(&k_date, &self.region);
        let k_service = hmac(&k_region, "s3");
        let k_signing = hmac(&k_service, "aws4_request");
        let signature = hex::encode(hmac(&k_signing, &to_sign));
        vec![
            ("x-amz-date".into(), amz_date),
            ("x-amz-content-sha256".into(), payload_hash.into()),
            (
                "authorization".into(),
                format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}", self.access_key),
            ),
        ]
    }

    /// Requête signée ; renvoie la réponse 2xx, ou l'erreur S3.
    pub async fn request(
        &self,
        method: &str,
        bucket: Option<&str>,
        key: Option<&str>,
        query: &[(&str, String)],
        headers: &[(String, String)],
        body: Option<Bytes>,
        timeout: Duration,
    ) -> Result<reqwest::Response> {
        let url = self.url(bucket, key, query);
        let hash = sha256_hex(body.as_deref().unwrap_or_default());
        let mut req = self.http.request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), url.clone()).timeout(timeout);
        for (k, v) in headers { req = req.header(k, v); }
        for (k, v) in self.sign(method, &url, headers, &hash) { req = req.header(k, v); }
        if let Some(b) = body { req = req.body(b); }
        let res = req.send().await.map_err(|e| S3Error {
            status: None,
            code: String::new(),
            message: e.to_string(),
            source: Some(if e.is_timeout() {
                SourceError::new(ErrorCode::Timeout, "Le S3 ne répond pas (délai dépassé).")
            } else {
                net::from_reqwest(&e, self.endpoint.as_str())
            }),
        })?;
        let status = res.status().as_u16();
        if (200..300).contains(&status) { return Ok(res); }
        let text = res.text().await.unwrap_or_default();
        let parsed: Option<XmlError> = quick_xml::de::from_str(&text).ok();
        let (code, message) = match parsed {
            Some(x) if !x.code.is_empty() => (x.code, x.message),
            _ if text.trim().is_empty() => (
                match status { 404 => "NotFound".into(), 403 => "Forbidden".into(), 401 => "Unauthorized".into(), _ => "UnknownError".into() },
                String::new(),
            ),
            _ => ("UnknownError".into(), String::new()),
        };
        Err(S3Error { status: Some(status), code, message, source: None })
    }

    pub async fn send(&self, method: &str, bucket: Option<&str>, key: Option<&str>, query: &[(&str, String)]) -> Result<reqwest::Response> {
        self.request(method, bucket, key, query, &[], None, Duration::from_secs(60)).await
    }

    pub async fn text(res: reqwest::Response) -> Result<String> {
        res.text().await.map_err(|e| S3Error { status: None, code: String::new(), message: e.to_string(), source: Some(SourceError::new(ErrorCode::Network, e.to_string())) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodage() {
        assert_eq!(uri_encode("a b/c+d é", true), "a%20b/c%2Bd%20%C3%A9");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
    }

    #[test]
    fn erreurs() {
        let e = |code: &str, status: u16| S3Error { status: Some(status), code: code.into(), message: String::new(), source: None };
        assert_eq!(to_source_error(&e("SignatureDoesNotMatch", 403), "").code, ErrorCode::Unauthorized);
        assert_eq!(to_source_error(&e("InvalidAccessKeyId", 403), "").code, ErrorCode::Unauthorized);
        let denied = to_source_error(&e("AccessDenied", 403), "logs");
        assert_eq!(denied.code, ErrorCode::Forbidden);
        assert!(denied.message.contains("(logs)"));
        assert_eq!(to_source_error(&e("NoSuchBucket", 404), "b").message, "Bucket b introuvable.");
        assert!(to_source_error(&e("UnknownError", 502), "").message.contains("URL est-elle bien"));
    }

    /// Vecteur de la documentation AWS (GET Object avec Range, us-east-1), de bout en bout.
    #[test]
    fn signature_v4_requete_complete() {
        let conn = S3Connection {
            id: "x".into(),
            name: "x".into(),
            url: "https://s3.amazonaws.com".into(),
            region: Some("us-east-1".into()),
            path_style: false,
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            tls: Default::default(),
            buckets: vec![],
        };
        let client = S3Client::new(&conn, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY").unwrap();
        let url = client.url(Some("examplebucket"), Some("test.txt"), &[]);
        assert_eq!(url.as_str(), "https://examplebucket.s3.amazonaws.com/test.txt");
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let headers = client.sign_at("GET", &url, &[("Range".into(), "bytes=0-9".into())], &sha256_hex(b""), now);
        let auth = headers.iter().find(|(k, _)| k == "authorization").unwrap();
        assert_eq!(
            auth.1,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        // Path-style avec requête : paramètres triés et encodés.
        let ps = S3Client { path_style: true, ..client };
        let u = ps.url(Some("b"), None, &[("prefix", "a b/".into()), ("list-type", "2".into())]);
        assert_eq!(u.as_str(), "https://s3.amazonaws.com/b?list-type=2&prefix=a%20b%2F");
    }

    /// Vecteur de la documentation AWS (GET Object, région us-east-1).
    #[test]
    fn signature_v4() {
        let k_date = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "20130524");
        let k = hmac(&hmac(&hmac(&k_date, "us-east-1"), "s3"), "aws4_request");
        let canonical = "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let to_sign = format!("AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n{}", sha256_hex(canonical.as_bytes()));
        assert_eq!(hex::encode(hmac(&k, &to_sign)), "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");
    }
}
