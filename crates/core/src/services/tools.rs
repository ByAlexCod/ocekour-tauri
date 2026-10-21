//! Déclaration des outils d'un service : lecture, écriture (signée et journalisée). Chaque
//! outil n'est visible que si son service est activé avec un jeton (et, pour l'écriture, si
//! l'écriture est autorisée).

use super::definition;
use super::http::ServiceHttp;
use crate::mcp::{Content, Tool, Visibility, fail, ok, tool};
use crate::schema::S;
use serde_json::Value;
use std::future::Future;
use std::sync::Arc;

/// Résultat d'une écriture : valeur renvoyée à l'agent et lien web pour le journal.
pub struct WriteResult {
    pub value: Value,
    pub url: Option<String>,
}

fn unavailable(app: &crate::app::App, service: &str, write: bool) -> Option<String> {
    let label = definition(service).label;
    if !app.accounts.available(service) {
        return Some(format!("{label} n'est pas disponible (URL, jeton ou service désactivé) : Ocekour → Paramètres → Comptes."));
    }
    if write && !app.accounts.writable(service) {
        return Some(format!("Écriture {label} désactivée : l'utilisateur peut l'autoriser dans Ocekour → Paramètres → Comptes."));
    }
    None
}

/// Outil de lecture.
pub fn read<F, Fut>(service: &'static str, name: &str, title: &str, description: &str, input: Vec<(&str, S)>, f: F) -> Tool
where
    F: Fn(Value, ServiceHttp) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let f = Arc::new(f);
    tool(name, &format!("{} : {title}", definition(service).label), description)
        .input(input)
        .read_only()
        .visible(Visibility::Service(service, false))
        .run(move |app, args, _| {
            let f = f.clone();
            async move {
                if let Some(refused) = unavailable(&app, service, false) { return Ok(fail(refused)); }
                let http = app.accounts.client(service)?;
                Ok(ok(f(args, http).await?))
            }
        })
}

/// Outil d'écriture : signé, journalisé (cible lisible : `target`).
pub fn write<F, Fut, T>(service: &'static str, name: &str, title: &str, description: &str, input: Vec<(&str, S)>, target: T, destructive: bool, f: F) -> Tool
where
    F: Fn(Value, ServiceHttp) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<WriteResult>> + Send + 'static,
    T: Fn(&Value) -> anyhow::Result<String> + Send + Sync + 'static,
{
    let (f, target) = (Arc::new(f), Arc::new(target));
    let tool_name = name.to_string();
    let mut t = tool(
        name,
        &format!("{} : {title}", definition(service).label),
        &format!("{description} Écriture signée « via Ocekour » et inscrite au journal des actions."),
    )
    .input(input)
    .visible(Visibility::Service(service, true));
    if destructive { t = t.destructive(); }
    t.run(move |app, args, call| {
        let (f, target, tool_name) = (f.clone(), target.clone(), tool_name.clone());
        async move {
            if let Some(refused) = unavailable(&app, service, true) { return Ok::<Content, anyhow::Error>(fail(refused)); }
            // Cible illisible : le journal garde les paramètres.
            let target = target(&args).unwrap_or_default();
            let http = app.accounts.client(service)?;
            match f(args.clone(), http).await {
                Ok(result) => {
                    app.journal.add(call.client, service, &tool_name, &target, args, result.url.clone(), "ok", None);
                    log::info!("[mcp:{tool_name}] {target} {}", result.url.unwrap_or_default());
                    Ok(ok(result.value))
                }
                Err(e) => {
                    app.journal.add(call.client, service, &tool_name, &target, args, None, "error", Some(e.to_string()));
                    Err(e)
                }
            }
        }
    })
}

/// Tous les outils des services.
pub fn all() -> Vec<Tool> {
    let mut tools = super::gitlab::tools();
    tools.extend(super::jira::tools());
    tools.extend(super::confluence::tools());
    tools.extend(super::rocketchat::tools());
    tools
}
