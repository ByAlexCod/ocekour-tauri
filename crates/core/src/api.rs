//! Canaux de l'interface (mêmes noms que la version Electron) : l'interface Tauri appelle
//! `call(canal, arguments)`. Tant que l'application n'est pas activée, seuls les canaux de
//! l'écran d'activation répondent. Les arguments ne sont jamais journalisés (secrets).

use crate::analytics::{AnalyticsEvent, track, track_page};
use crate::app::App;
use crate::config::{SERVICE_IDS, Tls};
use crate::error::{Outcome, SourceError};
use crate::logs::parser::Level;
use crate::logs::{ExportFormat, LogEntry, SearchOptions, export_content, export_file_name};
use crate::mcp::logs::{ReadRequest, read_logs};
use crate::s3::S3Entry;
use crate::s3::service::S3Op;
use crate::streams::{StreamOptions, StreamState};
use crate::ui::{FileFilter, OpenOptions};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// Fenêtre à l'origine de l'appel (destinataire des flux et progressions).
#[derive(Debug, Clone, Default)]
pub struct Caller {
    pub window: String,
}

type R = Result<Value, String>;

fn arg(args: &[Value], i: usize) -> &Value { args.get(i).unwrap_or(&Value::Null) }

fn string(args: &[Value], i: usize, what: &str, max: usize) -> Result<String, String> {
    match arg(args, i).as_str() {
        Some(s) if s.chars().count() <= max => Ok(s.to_string()),
        Some(_) => Err(format!("{what} : trop long")),
        None => Err(format!("{what} : texte attendu")),
    }
}

fn non_empty(args: &[Value], i: usize, what: &str) -> Result<String, String> {
    let s = string(args, i, what, 100_000)?;
    if s.is_empty() { Err(format!("{what} : valeur vide")) } else { Ok(s) }
}

fn service_id(args: &[Value], i: usize) -> Result<String, String> {
    let id = string(args, i, "service", 50)?;
    if SERVICE_IDS.contains(&id.as_str()) { Ok(id) } else { Err(format!("Service inconnu : {id}")) }
}

fn web_url(url: &str) -> Result<String, String> {
    if crate::schema::is_url(url) && (url.starts_with("http://") || url.starts_with("https://")) { Ok(url.into()) } else { Err("Lien http(s) attendu".into()) }
}

fn to_value<T: serde::Serialize>(v: T) -> R { serde_json::to_value(v).map_err(|e| e.to_string()) }
fn outcome<T: serde::Serialize>(r: Result<T, SourceError>) -> R { to_value(Outcome::from(r)) }
fn e<E: std::fmt::Display>(err: E) -> String { err.to_string() }

/// Canaux ouverts sans activation : écran d'activation, journal, statistiques.
fn open_without_licence(channel: &str) -> bool {
    channel.starts_with("licence:") || channel.starts_with("analytics:")
        || ["config:get", "app:logError", "app:showLogFile", "app:copyText", "app:readClipboard"].contains(&channel)
}

/// Appel d'un canal ; erreurs et résultats en échec (`{ ok: false }`) sont journalisés.
pub async fn call(app: &Arc<App>, caller: &Caller, channel: &str, args: Vec<Value>) -> R {
    if channel != "licence:state" && !open_without_licence(channel) {
        app.licence_loaded().await;
        if !app.licence.usable() { return Err("Ocekour n'est pas activé.".into()); }
    }
    let result = dispatch(app, caller, channel, &args).await;
    match &result {
        Ok(v) if v["ok"] == false && v["error"].is_object() => {
            log::warn!("[{channel}] {} : {}", v["error"]["code"].as_str().unwrap_or_default(), v["error"]["message"].as_str().unwrap_or_default());
            // Statistiques : le canal et le code seulement (le message peut nommer un serveur).
            track(AnalyticsEvent::new("Erreur", channel, v["error"]["code"].as_str()));
        }
        Err(message) => {
            log::error!("[{channel}] {message}");
            track(AnalyticsEvent::new("Erreur", channel, Some("exception")));
        }
        _ => {}
    }
    result
}

async fn dispatch(app: &Arc<App>, caller: &Caller, channel: &str, args: &[Value]) -> R {
    let config = &app.config;
    match channel {
        // --- Configuration --------------------------------------------------------------------
        "config:get" => to_value(config.state()),
        "config:readText" => config.read_text().map(Value::String).map_err(e),
        "config:validateText" => {
            let text = string(args, 0, "texte", 5_000_000)?;
            to_value(crate::config::parse_config(&text).err().unwrap_or_default())
        }
        "config:saveText" => to_value(config.save_text(&string(args, 0, "texte", 5_000_000)?)),
        "config:setPreferences" => to_value(crate::settings::set_preferences(config, arg(args, 0)).map_err(e)?),
        "config:setNotifications" => to_value(crate::settings::set_notifications(config, arg(args, 0)).map_err(e)?),
        "config:setContexte" => to_value(crate::settings::set_contexte(config, arg(args, 0)).map_err(e)?),
        "config:setFichiers" => to_value(crate::settings::set_fichiers(config, arg(args, 0)).map_err(e)?),
        "config:toggleFavorite" => to_value(crate::settings::toggle_favorite(config, &non_empty(args, 0, "favori")?).map_err(e)?),
        "config:editProfile" => to_value(crate::settings::edit_profile(config, &string(args, 0, "match", 1_000)?, arg(args, 1)).map_err(e)?),
        "config:saveFilterPreset" => to_value(crate::settings::save_filter_preset(config, arg(args, 0), arg(args, 1)).map_err(e)?),
        "config:deleteFilterPreset" => to_value(crate::settings::delete_filter_preset(config, arg(args, 0)).map_err(e)?),
        "config:renameFilterPreset" => to_value(crate::settings::rename_filter_preset(config, arg(args, 0), arg(args, 1)).map_err(e)?),

        // --- Application ----------------------------------------------------------------------
        "app:copyText" => { app.ui.copy_text(&string(args, 0, "texte", 5_000_000)?).map_err(e)?; Ok(Value::Null) }
        "app:readClipboard" => app.ui.read_clipboard().map(Value::String).map_err(e),
        "app:showConfigFile" => { app.ui.show_in_folder(&app.data_dir.join("config.yaml")).map_err(e)?; Ok(Value::Null) }
        "app:showLogFile" => { app.ui.show_in_folder(&crate::logging::log_file()).map_err(e)?; Ok(Value::Null) }
        "app:logError" => { log::warn!("[interface] {}", string(args, 0, "message", 10_000)?); Ok(Value::Null) }

        // --- Connexions -----------------------------------------------------------------------
        "connections:test" => outcome(app.connections.test(arg(args, 0)).await.map(|summary| json!({ "summary": summary }))),
        "connections:listClusters" => outcome(app.connections.list_clusters(arg(args, 0)).await),
        "connections:save" => { let id = app.connections.save(arg(args, 0)).map_err(e)?; app.s3.reset(); Ok(json!(id)) }
        "connections:delete" => { app.connections.delete(&non_empty(args, 0, "id")?).map_err(e)?; Ok(Value::Null) }
        "connections:hasSecret" => Ok(json!(app.connections.has_secret(&non_empty(args, 0, "id")?))),
        "connections:readCaFile" => to_value(crate::net::ca_file_info(&non_empty(args, 0, "chemin")?).map_err(|e| e.message)?),
        // --- Sonde : empreinte du certificat au premier contact ---------------------------------
        "sonde:fingerprint" => outcome(crate::sonde::client::fetch_fingerprint(&string(args, 0, "url", 2_000)?).await),

        // --- Fichiers du poste ------------------------------------------------------------------
        "files:list" => to_value(app.files.list()),
        "files:pending" => to_value(app.files.take_pending()),
        "files:pick" => {
            let picked = app.ui.open_dialog(OpenOptions {
                title: "Ouvrir un fichier de logs".into(),
                filters: vec![FileFilter::new("Logs", &["log", "txt", "out", "err", "json", "ndjson", "jsonl", "gz"]), FileFilter::new("Tous les fichiers", &["*"])],
                multiple: true,
                ..Default::default()
            }).await;
            Ok(json!(picked.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>()))
        }
        "files:touch" => to_value(app.files.touch(&string(args, 0, "chemin", 4_000)?).map_err(|e| e.message)?),
        "files:favorite" => to_value(app.files.set_favorite(&string(args, 0, "chemin", 4_000)?, arg(args, 1).as_bool().unwrap_or(true)).map_err(|e| e.message)?),
        "files:forget" => to_value(app.files.forget(&string(args, 0, "chemin", 4_000)?)),
        "files:setEncoding" => to_value(app.files.set_encoding(&string(args, 0, "chemin", 4_000)?, arg(args, 1).as_str()).map_err(|e| e.message)?),
        "connections:pickCaFile" => {
            let picked = app.ui.open_dialog(OpenOptions { title: "Certificat d'autorité".into(), filters: vec![FileFilter::new("Certificats", &["cer", "crt", "pem"])], ..Default::default() }).await;
            match picked.first() {
                Some(p) => to_value(crate::net::ca_file_info(&p.to_string_lossy()).map_err(|e| e.message)?),
                None => Ok(Value::Null),
            }
        }
        "connections:importKubeconfig" => {
            let picked = app.ui.open_dialog(OpenOptions { title: "Kubeconfig Rancher".into(), default_path: dirs::home_dir().map(|h| h.join(".kube")), ..Default::default() }).await;
            match picked.first() {
                Some(p) => to_value(app.connections.import_kubeconfig(&p.to_string_lossy()).map_err(e)?),
                None => Ok(Value::Null),
            }
        }

        // --- Bases de données -----------------------------------------------------------------
        "databases:list" => to_value(app.databases.list()),
        "databases:test" => to_value(app.databases.test(arg(args, 0)).await),
        "databases:save" => Ok(json!(app.databases.save(arg(args, 0)).map_err(e)?)),
        "databases:delete" => { app.databases.delete(&string(args, 0, "id", 200)?).map_err(e)?; Ok(Value::Null) }

        // --- Sources et flux ------------------------------------------------------------------
        "sources:browse" => outcome(crate::sources::browse_value(&app.connections, arg(args, 0)).await),
        "streams:open" => {
            let target = arg(args, 0);
            let id = target["connectionId"].as_str().filter(|s| !s.is_empty()).ok_or("connectionId attendu")?;
            let path: Vec<String> = serde_json::from_value(target["path"].clone()).map_err(|_| "Chemin de flux invalide".to_string())?;
            let options: StreamOptions = serde_json::from_value(arg(args, 1).clone()).map_err(|e| format!("Options de flux invalides : {e}"))?;
            let ui = app.ui.clone();
            let window = caller.window.clone();
            let send = Arc::new(move |id: &str, event: crate::streams::StreamEvent| ui.emit_to(&window, "streams:event", json!({ "id": id, "event": event })));
            Ok(json!(app.streams.open(&caller.window, id, &path, options, send).map_err(|e| e.message)?))
        }
        "streams:close" => { app.streams.close(&string(args, 0, "id", 100)?); Ok(Value::Null) }

        // --- S3 -------------------------------------------------------------------------------
        "s3:list" | "s3:preview" | "s3:count" | "s3:download" | "s3:upload" | "s3:rename" | "s3:delete" | "s3:cancel" => s3(app, caller, channel, args).await,

        // --- Licence et statistiques ----------------------------------------------------------
        "licence:state" => { app.licence_loaded().await; to_value(app.licence_state()) }
        "licence:activate" => {
            let key = string(args, 0, "clé", 100_000)?;
            Ok(match app.activate(&key).await {
                Ok(state) => json!({ "ok": true, "value": state }),
                Err(err) => json!({ "ok": false, "error": { "code": "invalid", "message": err.to_string() } }),
            })
        }
        "licence:retry" => to_value(app.with_build(app.licence.retry().await)),
        "analytics:event" => {
            let ev = arg(args, 0);
            let field = |k: &str, max: usize| ev[k].as_str().filter(|s| !s.is_empty() && s.chars().count() <= max).map(str::to_string);
            let (Some(category), Some(action)) = (field("category", 100), field("action", 100)) else { return Err("Événement invalide".into()) };
            let mut event = AnalyticsEvent::new(&category, &action, ev["name"].as_str().filter(|n| n.chars().count() <= 150));
            if let Some(v) = ev["value"].as_f64() { event = event.value(v); }
            track(event);
            Ok(Value::Null)
        }
        "analytics:page" => { track_page(&string(args, 0, "titre", 100)?, &string(args, 1, "chemin", 150)?); Ok(Value::Null) }
        "analytics:status" => Ok(json!({ "enabled": app.licence.analytics_target().is_some() })),

        // --- Comptes et journal des actions ---------------------------------------------------
        "services:list" => to_value(app.accounts.list()),
        "services:save" => {
            let id = service_id(args, 0)?;
            let settings = arg(args, 1);
            let url = settings["url"].as_str().filter(|u| u.len() <= 2_000).ok_or("URL attendue")?;
            let tls: Tls = serde_json::from_value(settings["tls"].clone()).map_err(|_| "TLS invalide".to_string())?;
            to_value(app.accounts.save(&id, url, &tls).map_err(e)?)
        }
        "services:setFlags" => {
            let id = service_id(args, 0)?;
            let flags = arg(args, 1);
            let (enabled, write) = (flags["enabled"].as_bool(), flags["write"].as_bool());
            if let Some(w) = write { track(AnalyticsEvent::new("Comptes", if w { "ecriture_activee" } else { "ecriture_desactivee" }, Some(&id))); }
            to_value(app.accounts.set_flags(&id, enabled, write).map_err(e)?)
        }
        "services:setToken" => {
            let id = service_id(args, 0)?;
            let token = string(args, 1, "jeton", 10_000)?;
            let user = arg(args, 2).as_str().filter(|u| u.len() <= 1_000).map(str::to_string);
            let result = app.accounts.set_token(&id, &token, user.as_deref()).await.map_err(e)?;
            track(AnalyticsEvent::new("Comptes", "jeton_enregistre", Some(&format!("{id} {}", if result.ok { "ok" } else { "erreur" }))));
            to_value(result)
        }
        "services:test" => {
            let id = service_id(args, 0)?;
            let result = app.accounts.test(&id).await;
            track(AnalyticsEvent::new("Comptes", "jeton_test", Some(&format!("{id} {}", if result.ok { "ok" } else { "erreur" }))));
            to_value(result)
        }
        "services:forgetToken" => to_value(app.accounts.forget_token(&service_id(args, 0)?).map_err(e)?),
        "services:openTokenPage" => {
            let id = service_id(args, 0)?;
            track(AnalyticsEvent::new("Comptes", "jeton_cree_ouvert", Some(&id)));
            let url = web_url(&app.accounts.token_page(&id, arg(args, 1).as_bool().unwrap_or(false)).map_err(e)?)?;
            app.ui.open_url(&url).map_err(e)?;
            Ok(Value::Null)
        }
        "actions:list" => {
            let q = arg(args, 0);
            let query = crate::services::journal::Query {
                offset: q["offset"].as_u64().map(|n| n as usize),
                limit: q["limit"].as_u64().map(|n| (n as usize).clamp(1, 1_000)),
                service: q["service"].as_str().map(str::to_string),
                status: q["status"].as_str().filter(|s| ["ok", "error"].contains(s)).map(str::to_string),
                text: q["text"].as_str().map(|t| t.chars().take(500).collect()),
            };
            let (entries, total) = app.journal.list(&query);
            Ok(json!({ "entries": entries, "total": total }))
        }
        "actions:openUrl" => { app.ui.open_url(&web_url(&string(args, 0, "lien", 4_000)?)?).map_err(e)?; Ok(Value::Null) }

        // --- Notifications, contexte IA, serveur MCP ------------------------------------------
        "notifications:status" => to_value(app.notifier.status()),
        "notifications:test" => to_value(app.notifier.test().await),
        "contexte:status" => to_value(app.context.status()),
        "contexte:detectLlm" => {
            let mode = config.config().contexte.llm.clone();
            Ok(json!(crate::contexte::llm::Curator::new(crate::contexte::llm::discover_here(&mode), Tls::default()).describe()))
        }
        "contexte:agents" => to_value(crate::contexte::agents::agent_states(&dirs::home_dir().unwrap_or_default())),
        "contexte:configureAgent" => {
            let tool = string(args, 0, "outil", 10)?;
            let action = string(args, 1, "action", 10)?;
            if !["qwen", "vibe"].contains(&tool.as_str()) || !["add", "remove"].contains(&action.as_str()) { return Err("Outil ou action inconnu".into()); }
            let home = dirs::home_dir().unwrap_or_default();
            let target = crate::contexte::agents::McpTarget { url: format!("http://127.0.0.1:{}/mcp", config.config().mcp_port), token: app.mcp_token(false) };
            crate::contexte::agents::configure_agent(&tool, action == "add", &target, &home).map_err(e)?;
            to_value(crate::contexte::agents::agent_states(&home))
        }
        "mcp:status" => to_value(app.mcp_status()),
        "mcp:regenerateToken" => { app.mcp_token(true); to_value(app.mcp_status()) }

        // --- Export ---------------------------------------------------------------------------
        "export:pick" => {
            let name = string(args, 0, "nom", 300)?;
            let Some(file) = app.ui.save_dialog("Exporter les lignes", app.ui.downloads_dir().join(format!("{name}.ndjson")), export_filters()).await else { return Ok(Value::Null) };
            let token = uuid::Uuid::new_v4().to_string();
            let format = ExportFormat::for_file(&file.to_string_lossy());
            app.approved_exports.lock().insert(token.clone(), file);
            Ok(json!({ "token": token, "format": format.as_str() }))
        }
        "export:write" => {
            let token = string(args, 0, "jeton", 100)?;
            let content = string(args, 1, "contenu", usize::MAX)?;
            let file = app.approved_exports.lock().remove(&token).ok_or("Export expiré : recommencez.")?;
            tokio::fs::write(&file, content).await.map_err(e)?;
            Ok(Value::Null)
        }
        "export:downloadLogs" => download_logs(app, arg(args, 0)).await,

        other => Err(format!("Canal inconnu : {other}")),
    }
}

fn export_filters() -> Vec<FileFilter> {
    vec![FileFilter::new("NDJSON (une ligne JSON par log)", &["ndjson", "jsonl", "json"]), FileFilter::new("Texte brut", &["log", "txt"])]
}

fn s3_ref(v: &Value) -> Result<(String, String), String> {
    match (v["connectionId"].as_str().filter(|s| !s.is_empty()), v["bucket"].as_str().filter(|s| !s.is_empty())) {
        (Some(c), Some(b)) => Ok((c.into(), b.into())),
        _ => Err("Référence S3 invalide".into()),
    }
}

fn entries(v: &Value) -> Result<Vec<S3Entry>, String> {
    let list: Vec<S3Entry> = serde_json::from_value(v.clone()).map_err(|_| "Éléments S3 invalides".to_string())?;
    if list.is_empty() || list.iter().any(|e| e.key.is_empty() || !["file", "folder"].contains(&e.kind.as_str())) { return Err("Éléments S3 invalides".into()); }
    Ok(list)
}

async fn s3(app: &Arc<App>, caller: &Caller, channel: &str, args: &[Value]) -> R {
    if channel == "s3:cancel" {
        app.s3.cancel(&string(args, 0, "opération", 100)?);
        return Ok(Value::Null);
    }
    let (conn, bucket) = s3_ref(arg(args, 0))?;
    let sink = {
        let (ui, window) = (app.ui.clone(), caller.window.clone());
        Arc::new(move |p: crate::s3::service::S3Progress| ui.emit_to(&window, "s3:progress", json!(p)))
    };
    let start = |label: String, op: S3Op| -> R { Ok(json!(app.s3.start(&conn, &bucket, &label, op, sink.clone()).map_err(|e| e.message)?)) };
    match channel {
        "s3:list" => {
            let prefix = string(args, 1, "préfixe", 2_000)?;
            let o = arg(args, 2);
            outcome(app.s3.list(&conn, &bucket, &prefix, o["search"].as_str().unwrap_or_default(), o["continuation"].as_str()).await)
        }
        "s3:preview" => outcome(app.s3.preview(&conn, &bucket, &string(args, 1, "clé", 2_000)?).await),
        "s3:count" => outcome(app.s3.count(&conn, &bucket, &string(args, 1, "préfixe", 2_000)?).await),
        "s3:download" => {
            let list = entries(arg(args, 1))?;
            if list.len() == 1 && !list[0].is_folder() {
                let single = list[0].clone();
                let Some(file) = app.ui.save_dialog("Télécharger", app.ui.downloads_dir().join(&single.name), vec![]).await else { return Ok(Value::Null) };
                return start(format!("téléchargement de {}", single.key), S3Op::Download { entries: list, file: Some(file), dir: None });
            }
            let dirs = app.ui.open_dialog(OpenOptions { title: "Télécharger dans le dossier…".into(), directory: true, ..Default::default() }).await;
            let Some(dir) = dirs.into_iter().next() else { return Ok(Value::Null) };
            start(format!("téléchargement de {} élément(s)", list.len()), S3Op::Download { entries: list, file: None, dir: Some(dir) })
        }
        "s3:upload" => {
            let prefix = string(args, 1, "préfixe", 2_000)?;
            if !prefix.is_empty() && !prefix.ends_with('/') { return Err("Préfixe de dossier attendu".into()); }
            let paths: Vec<PathBuf> = match arg(args, 2) {
                Value::Array(list) if !list.is_empty() => list.iter().map(|p| p.as_str().filter(|s| !s.is_empty()).map(PathBuf::from).ok_or("Chemin invalide".to_string())).collect::<Result<_, _>>()?,
                Value::String(kind) if kind == "files" || kind == "folder" => {
                    let folder = kind == "folder";
                    let picked = app.ui.open_dialog(OpenOptions {
                        title: if folder { "Envoyer un dossier" } else { "Envoyer des fichiers" }.into(),
                        directory: folder,
                        multiple: !folder,
                        ..Default::default()
                    }).await;
                    if picked.is_empty() { return Ok(Value::Null); }
                    picked
                }
                _ => return Err("Source d'envoi invalide".into()),
            };
            start(format!("envoi de {} élément(s) vers /{prefix}", paths.len()), S3Op::Upload { prefix, paths })
        }
        "s3:rename" => {
            let entry: S3Entry = serde_json::from_value(arg(args, 1).clone()).map_err(|_| "Élément S3 invalide".to_string())?;
            let new_name = string(args, 2, "nom", 1_000)?;
            start(format!("renommage de {} en {new_name}", entry.key), S3Op::Rename { entry, new_name })
        }
        _ => {
            let list = entries(arg(args, 1))?;
            let what = list.iter().map(|x| x.key.as_str()).collect::<Vec<_>>().join(", ");
            start(format!("suppression de {what}"), S3Op::Remove { entries: list })
        }
    }
}

/// Lignes au plus écrites dans un téléchargement d'historique complet (garde-fou mémoire).
#[cfg(not(test))]
pub const DOWNLOAD_CAP: usize = 2_000_000;
#[cfg(test)]
pub const DOWNLOAD_CAP: usize = 1_000;

/// Historique complet d'un flux ou d'un assemblage : fusion par horodatage, écriture du
/// fichier choisi. Historique seul (jamais de suivi).
async fn download_logs(app: &Arc<App>, request: &Value) -> R {
    let default_name = request["defaultName"].as_str().filter(|n| !n.is_empty() && n.chars().count() <= 300).ok_or("Nom attendu")?;
    let filtered = request["filtered"].as_bool().ok_or("filtered attendu")?;
    let sources = request["sources"].as_array().filter(|s| !s.is_empty() && s.len() <= 10).ok_or("Sources invalides")?;
    let name = export_file_name(default_name, chrono::Local::now());
    let Some(file) = app.ui.save_dialog("Télécharger l’historique complet", app.ui.downloads_dir().join(format!("{name}.ndjson")), export_filters()).await else { return Ok(Value::Null) };
    let format = ExportFormat::for_file(&file.to_string_lossy());
    let search: Option<SearchOptions> = if filtered { serde_json::from_value(request["search"].clone()).ok() } else { None };
    let levels: Option<Vec<Level>> = if filtered {
        request["levels"].as_array().map(|l| l.iter().filter_map(|v| v.as_str().and_then(Level::parse)).collect::<Vec<_>>()).filter(|l| !l.is_empty())
    } else {
        None
    };
    let multi = sources.len() > 1;
    // Clé de tri de chaque ligne : son horodatage, ou celui de la ligne datée qui la précède
    // (suite d'une pile d'erreur, ligne multiligne) pour qu'elle reste à sa place à la fusion.
    let mut all: Vec<(i64, LogEntry)> = vec![];
    let (mut truncated, mut incomplete) = (false, vec![]);
    for source in sources {
        let connection_id = source["connectionId"].as_str().filter(|s| !s.is_empty()).ok_or("connectionId attendu")?.to_string();
        let path: Vec<String> = serde_json::from_value(source["path"].clone()).map_err(|_| "Chemin invalide".to_string())?;
        if path.is_empty() || path.len() > 5 { return Err("Chemin invalide".into()); }
        let label = source["label"].as_str().map(str::to_string).unwrap_or_else(|| path.last().cloned().unwrap_or_default());
        let encoding = source["encoding"].as_str().filter(|e| !e.is_empty()).map(str::to_string);
        let read = |previous: bool| ReadRequest {
            connection_id: connection_id.clone(),
            path: path.clone(),
            options: StreamOptions { follow: false, previous: previous.then_some(true), encoding: encoding.clone(), ..Default::default() },
            follow_seconds: 0,
            search: search.clone(),
            levels: levels.clone(),
            max_lines: DOWNLOAD_CAP,
            until_end: true,
        };
        let mut entries = vec![];
        // Instance précédente (avant redémarrage) d'abord : logs plus anciens. Absente : ignorée.
        if source["includePrevious"] == true {
            if let Ok(r) = read_logs(app, read(true)).await {
                if r.status == StreamState::Ended { truncated |= r.truncated; entries.extend(r.entries); }
            }
        }
        let r = read_logs(app, read(false)).await.map_err(e)?;
        // Source en erreur : le dire, plutôt qu'un fichier vide ou partiel présenté comme complet.
        let gone = r.replacements.as_ref().map(|pods| {
            let current = if pods.is_empty() { String::new() } else { format!(" Pods actuels : {}.", pods.join(", ")) };
            format!("{} Ses logs ne sont plus disponibles.{current}", r.message.clone().unwrap_or_default())
        });
        if r.status == StreamState::Error || gone.is_some() {
            let message = gone.or(r.message).unwrap_or_else(|| "lecture impossible".into());
            return Err(if multi { format!("{label} : {message}") } else { message });
        }
        if r.status != StreamState::Ended { incomplete.push(label.clone()); }
        truncated |= r.truncated;
        entries.extend(r.entries);
        if multi {
            for entry in &mut entries { entry.source = Some(label.clone()); }
        }
        let mut key = entries.iter().map(|e| e.time_key()).find(|k| *k != i64::MAX).unwrap_or(i64::MAX);
        for entry in entries {
            let own = entry.time_key();
            if own != i64::MAX { key = own; }
            all.push((key, entry));
        }
    }
    if multi { all.sort_by_key(|(key, _)| *key); }
    if all.len() > DOWNLOAD_CAP {
        truncated = true;
        all.drain(..all.len() - DOWNLOAD_CAP);
    }
    let all: Vec<LogEntry> = all.into_iter().map(|(_, entry)| entry).collect();
    tokio::fs::write(&file, export_content(&all, format)).await.map_err(e)?;
    log::info!("Téléchargement logs : {} ligne(s) → {}{}", all.len(), file.display(), if incomplete.is_empty() { String::new() } else { format!(" (incomplet : {})", incomplete.join(", ")) });
    Ok(json!({ "file": file, "lines": all.iter().filter(|e| !e.notice).count(), "truncated": truncated, "incomplete": incomplete }))
}
