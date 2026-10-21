//! L'application sans interface : tous les services, partagés par la fenêtre (Tauri), le
//! serveur MCP et le mode headless. L'interface n'est qu'une surcouche (trait `Ui`).

use crate::bdd::DatabaseManager;
use crate::analytics::{self, Analytics, AnalyticsEvent, track};
use crate::config::ConfigStore;
use crate::connections::{Connections, PresetSummary};
use crate::licence::{LicenceKeys, LicenceManager, LicenceState};
use crate::mcp::Tool;
use crate::mcp::http::McpHttpServer;
use crate::s3::service::S3Service;
use crate::secrets::{LocalEncryptor, SecretStore};
use crate::services::Accounts;
use crate::services::journal::ActionJournal;
use crate::streams::manager::StreamManager;
use crate::ui::Ui;
use crate::web::browser::WebHub;
use base64::Engine;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const MCP_TOKEN_ID: &str = "mcp:token";

/// Paramètres de démarrage.
pub struct AppOptions {
    pub data_dir: PathBuf,
    pub version: String,
    pub ui: Arc<dyn Ui>,
    /// User-Agent des statistiques (navigateur de la fenêtre).
    pub user_agent: Option<String>,
    pub language: Option<String>,
    pub resolution: Option<String>,
    /// Clés de licence (celles de la compilation par défaut).
    pub licence_keys: Option<LicenceKeys>,
}

impl AppOptions {
    pub fn new(data_dir: impl Into<PathBuf>, version: &str, ui: Arc<dyn Ui>) -> Self {
        AppOptions { data_dir: data_dir.into(), version: version.into(), ui, user_agent: None, language: None, resolution: None, licence_keys: None }
    }
}

/// État du serveur MCP HTTP (Paramètres → Serveur MCP).
#[derive(Debug, Clone, Serialize)]
pub struct McpStatus {
    pub enabled: bool,
    pub running: bool,
    pub url: String,
    pub token: String,
    pub executable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct App {
    pub version: String,
    pub data_dir: PathBuf,
    pub ui: Arc<dyn Ui>,
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
    pub connections: Arc<Connections>,
    pub streams: StreamManager,
    pub s3: S3Service,
    pub licence: Arc<LicenceManager>,
    pub analytics: Arc<Analytics>,
    pub accounts: Arc<Accounts>,
    pub journal: Arc<ActionJournal>,
    pub mcp_http: McpHttpServer,
    /// Navigateur piloté par les outils web_* (partagé entre les appels).
    pub web: WebHub,
    /// Bases de données en lecture seule (outils bdd_*).
    pub databases: DatabaseManager,
    mcp_error: Mutex<Option<String>>,
    tools: once_cell::sync::OnceCell<Arc<Vec<Tool>>>,
    licence_loading: tokio::sync::OnceCell<()>,
}

/// Dossier des données : %APPDATA%\Ocekour (~/.config/Ocekour), ou `OCEKOUR_DATA_DIR`.
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OCEKOUR_DATA_DIR").filter(|d| !d.is_empty()) { return dir.into(); }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("Ocekour")
}

fn legacy_encryptor(_data_dir: &Path) -> Option<Box<dyn crate::secrets::Encryptor>> {
    #[cfg(windows)]
    {
        let local_state = _data_dir.join("Local State");
        if local_state.exists() { return Some(Box::new(crate::secrets::legacy::ElectronSafeStorage::new(&local_state))); }
    }
    None
}

impl App {
    /// Construit l'application (dans un runtime tokio).
    pub fn new(options: AppOptions) -> anyhow::Result<Arc<App>> {
        crate::net::install_default_provider();
        let data_dir = options.data_dir;
        std::fs::create_dir_all(&data_dir)?;
        let config = ConfigStore::new(data_dir.join("config.yaml"));
        let secrets = Arc::new(SecretStore::new(
            data_dir.join("secrets.json"),
            Box::new(LocalEncryptor::new(data_dir.join("secret.key"))),
            legacy_encryptor(&data_dir),
        ));
        report_unreadable_secrets(&secrets);
        let connections = Arc::new(Connections::new(config.clone(), secrets.clone()));
        let accounts = Accounts::new(config.clone(), secrets.clone(), &format!("Ocekour/{} (agent MCP)", options.version));
        let journal = ActionJournal::new(data_dir.join("actions.ndjson"));
        let licence = LicenceManager::new(secrets.clone(), options.licence_keys.unwrap_or_else(LicenceKeys::of_build));
        let analytics = {
            let (target, delivery) = (licence.clone(), licence.clone());
            Analytics::new(analytics::Options {
                target: Box::new(move || target.analytics_target()),
                visitor_file: data_dir.join("statistiques-visiteur"),
                version: options.version.clone(),
                user_agent: options.user_agent.unwrap_or_else(|| format!("Mozilla/5.0 Ocekour/{}", options.version)),
                language: options.language.unwrap_or_else(|| "fr-FR".into()),
                resolution: options.resolution,
                flush: Duration::from_secs(30),
                heartbeat: Duration::from_secs(5 * 60),
                on_delivery: Some(Box::new(move |d| delivery.on_delivery(d))),
            })
        };
        analytics::install(analytics.clone());
        analytics.start();

        let app = Arc::new(App {
            version: options.version.clone(),
            ui: options.ui,
            streams: StreamManager::new(connections.clone()),
            s3: S3Service::new(connections.clone()),
            mcp_http: McpHttpServer::new({
                let secrets = secrets.clone();
                Arc::new(move || mcp_token(&secrets, false))
            }),
            mcp_error: Mutex::new(None),
            databases: DatabaseManager::new(config.clone(), secrets.clone()),
            web: WebHub::new(
                {
                    let config = config.clone();
                    Arc::new(move || config.config().web.clone())
                },
                &format!("Ocekour/{}", options.version),
            ),
            data_dir,
            config,
            secrets,
            connections,
            licence,
            analytics,
            accounts,
            journal,
            tools: once_cell::sync::OnceCell::new(),
            licence_loading: tokio::sync::OnceCell::new(),
        });

        // Événements vers l'interface.
        {
            let weak = Arc::downgrade(&app);
            app.licence.on_change(move |state| {
                log::info!(
                    "Licence : {}{}{}",
                    state.status,
                    state.info.as_ref().map(|i| format!(" — clé {} ({})", i.id, i.mode)).unwrap_or_default(),
                    state.message.as_ref().map(|m| format!(" — {m}")).unwrap_or_default()
                );
                if let Some(app) = weak.upgrade() { app.ui.emit("licence:changed", json!(app.with_build(state.clone()))); }
            });
            let weak = Arc::downgrade(&app);
            app.config.on_changed(move |state| {
                let Some(app) = weak.upgrade() else { return };
                app.s3.reset();
                if !state.errors.is_empty() { log::warn!("config.yaml invalide {}", json!(state.errors)); }
                app.ui.emit("config:changed", json!(state));
            });
            let weak = Arc::downgrade(&app);
            app.journal.on_added(move |action| {
                if let Some(app) = weak.upgrade() { app.ui.emit("actions:added", json!(action)); }
            });
        }
        Ok(app)
    }

    /// Clé lue à la première demande (aucun secret déchiffré avant).
    pub async fn licence_loaded(&self) {
        self.licence_loading
            .get_or_init(|| async {
                self.licence.load(self.analytics.clone()).await;
                track(AnalyticsEvent::new("Application", "Démarrage", Some(&self.version)));
            })
            .await;
    }

    pub fn with_build(&self, mut state: LicenceState) -> LicenceState {
        state.development = Some(self.licence.development());
        state
    }

    pub fn licence_state(&self) -> LicenceState { self.with_build(self.licence.state()) }

    /// Activation (écran d'activation ou outil MCP) : clé vérifiée puis préconfiguration.
    pub async fn activate(&self, key: &str) -> anyhow::Result<LicenceState> {
        if key.len() > 100_000 { anyhow::bail!("Clé trop longue."); }
        self.licence_loaded().await;
        let state = self.licence.activate(key).await?;
        track(AnalyticsEvent::new("Application", "Activation", state.info.as_ref().map(|i| i.mode.as_str())));
        // Préconfiguration de la clé : ajoutée à config.yaml (à chaque activation d'une clé).
        let preset: PresetSummary = self.connections.add_preset(&self.licence.preset())?;
        let mut state = self.with_build(state);
        state.preset = Some(serde_json::to_value(preset)?);
        Ok(state)
    }

    /// Outils MCP (déclarés une fois).
    pub fn mcp_tools(&self) -> Arc<Vec<Tool>> { self.tools.get_or_init(|| Arc::new(crate::mcp::tools::all())).clone() }

    pub fn mcp_token(&self, renew: bool) -> String { mcp_token(&self.secrets, renew) }

    /// Serveur MCP HTTP de l'interface : suit la préférence « mcpServer » et le port.
    pub async fn sync_mcp_http(self: &Arc<Self>) {
        let config = self.config.config();
        if config.mcp_server {
            let result = self.mcp_http.start(self.clone(), config.mcp_port).await;
            *self.mcp_error.lock() = result.err().map(|e| {
                let message = format!("Port {} indisponible : {e}", config.mcp_port);
                log::warn!("Serveur MCP : {message}");
                message
            });
        } else {
            self.mcp_http.stop();
            *self.mcp_error.lock() = None;
        }
    }

    /// Suivi de la préférence : démarre / arrête le serveur à chaque modification.
    pub fn watch_mcp_http(self: &Arc<Self>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        self.config.on_changed(move |_| { let _ = tx.send(()); });
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            if let Some(app) = weak.upgrade() { app.sync_mcp_http().await; }
            while rx.recv().await.is_some() {
                let Some(app) = weak.upgrade() else { break };
                app.sync_mcp_http().await;
            }
        });
    }

    pub fn mcp_status(&self) -> McpStatus {
        let config = self.config.config();
        McpStatus {
            enabled: config.mcp_server,
            running: self.mcp_http.running(),
            url: format!("http://127.0.0.1:{}/mcp", config.mcp_port),
            token: self.mcp_token(false),
            executable: std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            error: self.mcp_error.lock().clone(),
        }
    }

    /// Avant de quitter : dernier envoi des statistiques (2 s au plus), flux arrêtés.
    pub async fn shutdown(&self) {
        self.mcp_http.stop();
        self.licence.stop();
        self.web.dispose().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), self.analytics.stop()).await;
    }
}

/// Jeton du serveur MCP HTTP (coffre à secrets), créé au premier besoin.
pub fn mcp_token(secrets: &SecretStore, renew: bool) -> String {
    if !renew { if let Some(t) = secrets.get(MCP_TOKEN_ID) { return t; } }
    // Absent ou illisible (clé locale manquante) : on en régénère un. Les clients MCP
    // configurés avec l'ancien devront être reconfigurés (jeton à recopier).
    let bytes: [u8; 24] = rand::random();
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    if let Err(e) = secrets.set(MCP_TOKEN_ID, &token) {
        // Chiffrement indisponible : jeton valable pour cette session seulement.
        log::warn!("Jeton MCP non enregistré : {e}");
    }
    token
}

/// Au démarrage : signale les secrets devenus indéchiffrables, avec ce qu'il faut
/// ré-enregistrer. Le jeton MCP se régénère seul.
pub fn report_unreadable_secrets(secrets: &SecretStore) {
    let ids: Vec<String> = secrets.unreadable_ids().into_iter().filter(|id| id != MCP_TOKEN_ID).collect();
    if ids.is_empty() { return; }
    let mut what: Vec<String> = vec![];
    for id in &ids {
        let w = if id == crate::licence::LICENCE_SECRET_ID {
            "la clé d’activation (écran de licence)".to_string()
        } else if let Some(s) = id.strip_prefix("service:") {
            format!("le jeton {s} (Paramètres → Comptes)")
        } else {
            format!("un identifiant de connexion ({id})")
        };
        if !what.contains(&w) { what.push(w); }
    }
    log::warn!(
        "Des secrets enregistrés ne sont plus déchiffrables (fichier de clé secret.key manquant, ou anciens secrets Windows écrits sous une autre session). À ré-enregistrer : {}. Le jeton MCP, lui, est régénéré automatiquement.",
        what.join(" ; ")
    );
}
