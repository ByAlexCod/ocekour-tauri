//! Rancher 2.x : API Rancher (`/v3`) et API Kubernetes proxifiée (`/k8s/clusters/<id>`).
//! Navigation cluster › namespace › pod › conteneur.

use crate::config::Tls;
use crate::error::{ErrorCode, SourceError};
use crate::net;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

/// Cible réseau d'une connexion Rancher.
#[derive(Debug, Clone)]
pub struct RancherTarget {
    pub url: String,
    pub tls: Tls,
    pub token: String,
}

/// URL absolue d'un chemin relatif à l'URL de Rancher.
pub fn join(base: &str, path: &str) -> Result<url::Url, SourceError> {
    let base = format!("{}/", base.trim_end_matches('/'));
    url::Url::parse(&base)
        .and_then(|b| b.join(path))
        .map_err(|_| SourceError::new(ErrorCode::Network, format!("URL invalide : {base}")))
}

fn status_error(status: u16, body: &str, path: &str) -> SourceError {
    match status {
        401 => SourceError::new(ErrorCode::Unauthorized, "Token refusé par Rancher (expiré ou révoqué)."),
        403 => SourceError::new(ErrorCode::Forbidden, format!("Accès refusé (403) sur {path}.")),
        _ => {
            let message = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .filter(|m| !m.is_empty());
            SourceError::new(ErrorCode::Http, message.unwrap_or_else(|| format!("Rancher a répondu {status} sur {path}.")))
        }
    }
}

/// GET HTTPS avec le token et le TLS de la connexion ; réponse 2xx ou erreur typée.
pub async fn request(target: &RancherTarget, path: &str, timeout: Duration) -> Result<reqwest::Response, SourceError> {
    let url = join(&target.url, path)?;
    let client = net::http_client(&target.tls)?;
    let fut = client
        .get(url.clone())
        .bearer_auth(&target.token)
        .header("Accept", "application/json")
        .send();
    let res = match tokio::time::timeout(timeout, fut).await {
        Err(_) => {
            return Err(SourceError::new(
                ErrorCode::Timeout,
                format!("Pas de réponse de {} après {} s.", net::host_of(url.as_str()), timeout.as_secs()),
            ));
        }
        Ok(r) => r.map_err(|e| net::from_reqwest(&e, &target.url))?,
    };
    let status = res.status().as_u16();
    if (200..300).contains(&status) { return Ok(res); }
    let body = res.text().await.unwrap_or_default();
    Err(status_error(status, &body, url.path()))
}

/// GET JSON sur l'API Rancher.
pub async fn get_json(target: &RancherTarget, path: &str) -> Result<Value, SourceError> {
    let res = request(target, path, Duration::from_secs(15)).await?;
    let body = res.text().await.map_err(|e| net::from_reqwest(&e, &target.url))?;
    serde_json::from_str(&body)
        .map_err(|_| SourceError::new(ErrorCode::NotRancher, format!("{} ne répond pas comme une API Rancher.", target.url)))
}

pub async fn test_connection(target: &RancherTarget) -> Result<String, SourceError> {
    let me = get_json(target, "v3/users?me=true").await?;
    let user = me["data"].get(0).ok_or_else(|| SourceError::new(ErrorCode::NotRancher, "Réponse inattendue de /v3/users."))?;
    Ok(non_empty(&user["username"]).or(non_empty(&user["name"])).unwrap_or("?").to_string())
}

fn non_empty(v: &Value) -> Option<&str> { v.as_str().filter(|s| !s.is_empty()) }

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RancherCluster {
    pub id: String,
    pub name: String,
    pub state: String,
}

pub async fn list_clusters(target: &RancherTarget) -> Result<Vec<RancherCluster>, SourceError> {
    let clusters = get_json(target, "v3/clusters").await?;
    let mut list: Vec<RancherCluster> = clusters["data"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|c| {
            let id = c["id"].as_str().unwrap_or_default().to_string();
            RancherCluster {
                name: non_empty(&c["name"]).map(str::to_string).unwrap_or_else(|| id.clone()),
                state: c["state"].as_str().unwrap_or("unknown").to_string(),
                id,
            }
        })
        .collect();
    list.sort_by(|a, b| crate::text::locale_cmp(&a.name, &b.name));
    Ok(list)
}

// --- API Kubernetes ---------------------------------------------------------------------------

/// Erreur Kubernetes (statut HTTP) → erreur typée.
fn kube_error(status: u16) -> SourceError {
    match status {
        401 => SourceError::new(ErrorCode::Unauthorized, "Token refusé par Rancher (expiré ou révoqué)."),
        403 => SourceError::new(ErrorCode::Forbidden, "Accès refusé (403) : droits insuffisants."),
        404 => SourceError::new(ErrorCode::Http, "Élément introuvable (supprimé entre-temps ?)."),
        s => SourceError::new(ErrorCode::Http, format!("Kubernetes a répondu {s}.")),
    }
}

/// Résultat d'un appel Kubernetes : la valeur, ou le statut (404…) et l'erreur typée.
#[derive(Debug)]
pub struct KubeError {
    pub status: Option<u16>,
    pub error: SourceError,
}

impl From<KubeError> for SourceError {
    fn from(e: KubeError) -> Self { e.error }
}

pub async fn kube_get(target: &RancherTarget, cluster: &str, api_path: &str) -> Result<Value, KubeError> {
    let path = format!("k8s/clusters/{}{}", enc(cluster), api_path);
    let url = join(&target.url, &path).map_err(|error| KubeError { status: None, error })?;
    let client = net::http_client(&target.tls).map_err(|error| KubeError { status: None, error })?;
    let fut = client.get(url).bearer_auth(&target.token).header("Accept", "application/json").send();
    let res = match tokio::time::timeout(Duration::from_secs(30), fut).await {
        Err(_) => {
            return Err(KubeError {
                status: None,
                error: SourceError::new(ErrorCode::Timeout, format!("Pas de réponse de {} (délai dépassé).", net::host_of(&target.url))),
            });
        }
        Ok(r) => r.map_err(|e| KubeError { status: None, error: net::from_reqwest(&e, &target.url) })?,
    };
    let status = res.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(KubeError { status: Some(status), error: kube_error(status) });
    }
    res.json::<Value>().await.map_err(|e| KubeError { status: None, error: net::from_reqwest(&e, &target.url) })
}

/// Segment d'URL encodé.
pub fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// Élément de l'arborescence d'une source.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceNode {
    pub id: String,
    pub kind: &'static str,
    pub expandable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restarts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_previous: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partitions: Option<u64>,
}

/// Workload propriétaire d'un pod (Deployment déduit du ReplicaSet, CronJob du Job).
pub fn workload_of(pod: &Value) -> String {
    let meta = &pod["metadata"];
    let owners = meta["ownerReferences"].as_array().map(|a| a.as_slice()).unwrap_or_default();
    let owner = owners.iter().find(|o| o["controller"].as_bool() == Some(true)).or(owners.first());
    let Some(owner) = owner else { return meta["name"].as_str().unwrap_or_default().to_string() };
    let name = owner["name"].as_str().unwrap_or_default();
    let kind = owner["kind"].as_str().unwrap_or_default();
    if let Some(hash) = meta["labels"]["pod-template-hash"].as_str() {
        if kind == "ReplicaSet" && !hash.is_empty() && name.ends_with(&format!("-{hash}")) {
            return name[..name.len() - hash.len() - 1].to_string();
        }
    }
    if kind == "Job" {
        static JOB: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"-\d{8,}$").unwrap());
        return JOB.replace(name, "").to_string();
    }
    name.to_string()
}

fn statuses(pod: &Value) -> Vec<&Value> {
    let st = &pod["status"];
    st["initContainerStatuses"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(st["containerStatuses"].as_array().into_iter().flatten())
        .collect()
}

fn pod_node(pod: &Value) -> SourceNode {
    let all = statuses(pod);
    let waiting = all.iter().filter_map(|s| s["state"]["waiting"]["reason"].as_str()).find(|r| !r.is_empty());
    let terminated_error = all
        .iter()
        .map(|s| &s["state"]["terminated"])
        .find(|t| t.is_object() && t["exitCode"].as_i64() != Some(0))
        .and_then(|t| t["reason"].as_str());
    let phase = pod["status"]["phase"].as_str().unwrap_or("Unknown");
    let state = if !pod["metadata"]["deletionTimestamp"].is_null() {
        "Terminating".to_string()
    } else {
        waiting
            .or(terminated_error)
            .or(pod["status"]["reason"].as_str())
            .unwrap_or(phase)
            .to_string()
    };
    let ready = pod["status"]["containerStatuses"].as_array().into_iter().flatten().all(|s| s["ready"].as_bool() == Some(true));
    static BAD: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new("CrashLoop|Err|Error|OOM|BackOff|Failed|Evicted").unwrap());
    let health = if BAD.is_match(&state) {
        "error"
    } else if phase == "Succeeded" {
        "off"
    } else if state != "Running" || !ready {
        "warn"
    } else {
        "ok"
    };
    SourceNode {
        id: pod["metadata"]["name"].as_str().unwrap_or_default().to_string(),
        kind: "pod",
        expandable: true,
        health: Some(health),
        state: Some(state),
        workload: Some(workload_of(pod)),
        restarts: Some(all.iter().map(|s| s["restartCount"].as_u64().unwrap_or(0)).sum()),
        created_at: pod["metadata"]["creationTimestamp"].as_str().and_then(crate::text::to_iso),
        ..Default::default()
    }
}

fn container_nodes(pod: &Value) -> Vec<SourceNode> {
    let all = statuses(pod);
    let node = |name: &str, init: bool| {
        let s = all.iter().find(|s| s["name"].as_str() == Some(name));
        let state_of = |s: &Value| -> String {
            if s["state"]["running"].is_object() {
                "Running".into()
            } else {
                s["state"]["waiting"]["reason"]
                    .as_str()
                    .or(s["state"]["terminated"]["reason"].as_str())
                    .unwrap_or("Waiting")
                    .into()
            }
        };
        let state = s.map(|s| state_of(s)).unwrap_or_else(|| "Waiting".into());
        let health = match s {
            Some(s) if s["state"]["terminated"].is_object() => {
                if s["state"]["terminated"]["exitCode"].as_i64() == Some(0) { "off" } else { "error" }
            }
            Some(s) if s["state"]["waiting"].is_object() => {
                if state.contains("BackOff") || state.contains("Err") { "error" } else { "warn" }
            }
            _ => "ok",
        };
        SourceNode {
            id: name.to_string(),
            kind: "container",
            expandable: false,
            init: Some(init),
            state: Some(state),
            health: Some(health),
            restarts: Some(s.and_then(|s| s["restartCount"].as_u64()).unwrap_or(0)),
            has_previous: Some(s.map(|s| s["lastState"]["terminated"].is_object()).unwrap_or(false)),
            ..Default::default()
        }
    };
    let names = |key: &str| -> Vec<String> {
        pod["spec"][key].as_array().into_iter().flatten().filter_map(|c| c["name"].as_str().map(str::to_string)).collect()
    };
    names("initContainers")
        .iter()
        .map(|n| node(n, true))
        .chain(names("containers").iter().map(|n| node(n, false)))
        .collect()
}

/// Enfants d'un chemin Rancher : `[cluster]` → namespaces, `[cluster, ns]` → pods,
/// `[cluster, ns, pod]` → conteneurs.
pub async fn browse(target: &RancherTarget, path: &[String]) -> Result<Vec<SourceNode>, SourceError> {
    let (Some(cluster), rest) = (path.first(), &path[path.len().min(1)..]) else {
        return Err(SourceError::invalid("Chemin Rancher incomplet."));
    };
    if rest.len() > 2 { return Err(SourceError::invalid("Chemin invalide")); }
    let mut nodes: Vec<SourceNode> = match rest {
        [] => {
            let list = kube_get(target, cluster, "/api/v1/namespaces").await?;
            items(&list)
                .iter()
                .map(|ns| {
                    let state = ns["status"]["phase"].as_str().unwrap_or("Active").to_string();
                    SourceNode {
                        id: ns["metadata"]["name"].as_str().unwrap_or_default().into(),
                        kind: "namespace",
                        expandable: true,
                        health: if state == "Active" { None } else { Some("warn") },
                        state: Some(state),
                        ..Default::default()
                    }
                })
                .collect()
        }
        [ns] => {
            let list = kube_get(target, cluster, &format!("/api/v1/namespaces/{}/pods", enc(ns))).await?;
            items(&list).iter().map(pod_node).collect()
        }
        [ns, pod] => {
            let pod = kube_get(target, cluster, &format!("/api/v1/namespaces/{}/pods/{}", enc(ns), enc(pod))).await?;
            return Ok(container_nodes(&pod));
        }
        _ => unreachable!(),
    };
    nodes.sort_by(|a: &SourceNode, b| crate::text::locale_cmp(&a.id, &b.id));
    Ok(nodes)
}

pub fn items(list: &Value) -> &[Value] {
    list["items"].as_array().map(|a| a.as_slice()).unwrap_or_default()
}

// --- Kubeconfig -------------------------------------------------------------------------------

/// Brouillon issu d'un kubeconfig téléchargé depuis Rancher.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RancherDraft {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    pub tls: Tls,
    pub clusters: Vec<crate::config::ClusterRef>,
}

/// Kubeconfig téléchargé depuis Rancher → brouillon de connexion (URL, token, clusters).
/// Seuls les contexts qui passent par le proxy Rancher sont retenus.
pub fn draft_from_kubeconfig(text: &str) -> anyhow::Result<RancherDraft> {
    let raw = crate::yaml::parse(text).map_err(|e| anyhow::anyhow!(e.message))?.to_json();
    let named = |key: &str| raw[key].as_array().cloned().unwrap_or_default();
    let clusters: Vec<(String, String)> = named("clusters")
        .iter()
        .map(|c| (c["name"].as_str().unwrap_or_default().to_string(), c["cluster"]["server"].as_str().unwrap_or_default().to_string()))
        .collect();
    let users: Vec<(String, Option<String>)> = named("users")
        .iter()
        .map(|u| (u["name"].as_str().unwrap_or_default().to_string(), u["user"]["token"].as_str().map(str::to_string)))
        .collect();
    static RANCHER: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"^(https?://.+?)/k8s/clusters/([^/]+)/?$").unwrap());
    let mut url = String::new();
    let mut token: Option<String> = None;
    let mut found: Vec<crate::config::ClusterRef> = vec![];
    for ctx in named("contexts") {
        let cluster = ctx["context"]["cluster"].as_str().unwrap_or_default();
        let server = clusters.iter().find(|(n, _)| n == cluster).map(|(_, s)| s.as_str()).unwrap_or_default();
        let Some(m) = RANCHER.captures(server) else { continue };
        if !url.is_empty() && m[1] != url { continue; }
        url = m[1].to_string();
        if token.is_none() {
            let user = ctx["context"]["user"].as_str().unwrap_or_default();
            token = users.iter().find(|(n, _)| n == user).and_then(|(_, t)| t.clone());
        }
        if !found.iter().any(|c| c.id == m[2]) {
            found.push(crate::config::ClusterRef { id: m[2].to_string(), name: ctx["name"].as_str().unwrap_or_default().into() });
        }
    }
    if url.is_empty() {
        anyhow::bail!("Ce kubeconfig ne contient aucun cluster accessible via Rancher (…/k8s/clusters/<id>).");
    }
    Ok(RancherDraft {
        kind: "rancher",
        name: url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default(),
        url,
        secret: token,
        tls: Tls::default(),
        clusters: found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workload() {
        let pod = |kind: &str, name: &str, labels: Value| json!({ "metadata": { "name": "p", "labels": labels, "ownerReferences": [{ "kind": kind, "name": name, "controller": true }] } });
        assert_eq!(workload_of(&pod("ReplicaSet", "api-5f7c9", json!({ "pod-template-hash": "5f7c9" }))), "api");
        assert_eq!(workload_of(&pod("StatefulSet", "db", json!({}))), "db");
        assert_eq!(workload_of(&pod("Job", "purge-28765432", json!({}))), "purge");
        assert_eq!(workload_of(&json!({ "metadata": { "name": "solo" } })), "solo");
    }

    #[test]
    fn kubeconfig() {
        let draft = draft_from_kubeconfig(
            r#"
apiVersion: v1
kind: Config
clusters:
  - name: prod
    cluster:
      server: "https://rancher.intra/k8s/clusters/c-prod1"
  - name: prod-direct
    cluster:
      server: "https://10.0.0.1:6443"
users:
  - name: prod
    user:
      token: "kubeconfig-u-1:abc"
contexts:
  - name: prod
    context: { cluster: prod, user: prod }
  - name: prod-direct
    context: { cluster: prod-direct, user: prod }
current-context: prod
"#,
        )
        .unwrap();
        assert_eq!(draft.url, "https://rancher.intra");
        assert_eq!(draft.secret.as_deref(), Some("kubeconfig-u-1:abc"));
        assert_eq!(draft.clusters, vec![crate::config::ClusterRef { id: "c-prod1".into(), name: "prod".into() }]);
        assert_eq!(draft.name, "rancher.intra");
        assert!(draft_from_kubeconfig("clusters: []\ncontexts: []\n").unwrap_err().to_string().contains("Rancher"));
    }
}
