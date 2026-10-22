//! Commande `call` de la fenêtre : l'interface atteint le cœur par le pont IPC de Tauri
//! (runtime simulé, sans affichage).

use ocekour_core::app::{App, AppOptions};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::{Manager, WebviewWindowBuilder};

fn window(app: Arc<App>) -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>) {
    let tauri_app = mock_builder()
        .manage(app)
        .invoke_handler(ocekour_app::invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("application simulée");
    let window = WebviewWindowBuilder::new(&tauri_app, ocekour_app::MAIN_WINDOW, Default::default()).build().unwrap();
    (tauri_app, window)
}

fn call(window: &tauri::WebviewWindow<MockRuntime>, channel: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: "call".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // Origine du front embarqué (tauri://localhost ; http://tauri.localhost sous Windows).
            url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({ "channel": channel, "args": args })),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

#[test]
fn canaux_du_coeur_par_ipc() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    tauri::async_runtime::set(runtime.handle().clone());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.yaml"), "# test\ntheme: dark\n").unwrap();
    let app = App::new(AppOptions::new(dir.path(), "2.0.0-test", Arc::new(ocekour_core::ui::NoUi))).unwrap();
    let (tauri_app, window) = window(app);
    assert!(tauri_app.state::<Arc<App>>().config.config().theme == "dark");

    // Canaux ouverts sans activation.
    let config = call(&window, "config:get", json!([])).unwrap();
    assert_eq!(config["config"]["theme"], "dark");
    let licence = call(&window, "licence:state", json!([])).unwrap();
    assert_eq!(licence["status"], "missing");

    // Sans activation, les autres canaux sont refusés par le cœur avec un message lisible.
    let refused = call(&window, "config:setPreferences", json!([{ "theme": "light" }])).unwrap_err();
    assert_eq!(refused, json!("Ocekour n'est pas activé."));
    assert!(std::fs::read_to_string(dir.path().join("config.yaml")).unwrap().contains("theme: dark"));

    // Canal inconnu : erreur, pas de panique.
    assert!(call(&window, "inexistant:canal", json!([])).is_err());
}
