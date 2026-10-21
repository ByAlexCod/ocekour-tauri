//! Hub de navigation : un navigateur du poste (Edge / Chrome / Chromium) piloté par le
//! protocole DevTools, lancé paresseusement au premier usage et partagé entre les appels MCP.
//! Aucun binaire de navigateur téléchargé.

use super::cdp::{Cdp, EventHandler};
use super::snapshot::{COLLECT_ELEMENTS, SnapElement, format_snapshot};
use crate::config::WebConfig;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TabInfo {
    pub index: usize,
    pub actif: bool,
    pub titre: String,
    pub url: String,
}

#[derive(Debug, Clone)]
struct Page {
    target: String,
    session: String,
}

struct Browser {
    child: tokio::process::Child,
    cdp: Arc<Cdp>,
    pages: Vec<Page>,
    active: usize,
    profile: PathBuf,
    timeout: Duration,
}

pub type WebConfigFn = Arc<dyn Fn() -> WebConfig + Send + Sync>;

pub struct WebHub {
    config: WebConfigFn,
    pub user_agent: String,
    browser: Mutex<Option<Browser>>,
}

/// Exécutables candidats pour un canal (msedge, chrome, chromium).
pub fn candidates(channel: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    if let Some(p) = std::env::var_os("OCEKOUR_BROWSER_PATH").filter(|p| !p.is_empty()) { out.push(p.into()); }
    #[cfg(windows)]
    {
        let roots: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var_os(v)).map(PathBuf::from).collect();
        let rel: &[&str] = match channel {
            "msedge" => &[r"Microsoft\Edge\Application\msedge.exe"],
            "chrome" => &[r"Google\Chrome\Application\chrome.exe"],
            _ => &[r"Chromium\Application\chrome.exe"],
        };
        for root in &roots { for r in rel { out.push(root.join(r)); } }
    }
    #[cfg(target_os = "macos")]
    {
        let app = match channel {
            "msedge" => "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "chrome" => "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            _ => "/Applications/Chromium.app/Contents/MacOS/Chromium",
        };
        out.push(app.into());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let names: &[&str] = match channel {
            "msedge" => &["microsoft-edge", "microsoft-edge-stable"],
            "chrome" => &["google-chrome", "google-chrome-stable"],
            _ => &["chromium", "chromium-browser"],
        };
        for dir in std::env::var("PATH").unwrap_or_default().split(':') {
            for n in names { out.push(PathBuf::from(dir).join(n)); }
        }
        if channel == "chromium" { out.push("/opt/pw-browsers/chromium".into()); }
    }
    out
}

fn running_as_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .map(|s| s.lines().any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0")))
        .unwrap_or(false)
}

fn selector(r: &str) -> String { format!("[data-ocekour-ref=\"{}\"]", r.replace(['"', '\\'], "")) }

/// Touche (nom Playwright) → (key, code, keyCode, texte).
fn key_def(name: &str) -> anyhow::Result<(String, String, i64, Option<String>)> {
    let named: Option<(&str, i64, Option<&str>)> = match name {
        "Enter" => Some(("Enter", 13, Some("\r"))),
        "Escape" => Some(("Escape", 27, None)),
        "Tab" => Some(("Tab", 9, None)),
        "Backspace" => Some(("Backspace", 8, None)),
        "Delete" => Some(("Delete", 46, None)),
        "PageUp" => Some(("PageUp", 33, None)),
        "PageDown" => Some(("PageDown", 34, None)),
        "End" => Some(("End", 35, None)),
        "Home" => Some(("Home", 36, None)),
        "ArrowLeft" => Some(("ArrowLeft", 37, None)),
        "ArrowUp" => Some(("ArrowUp", 38, None)),
        "ArrowRight" => Some(("ArrowRight", 39, None)),
        "ArrowDown" => Some(("ArrowDown", 40, None)),
        "Insert" => Some(("Insert", 45, None)),
        "Space" | " " => return Ok((" ".into(), "Space".into(), 32, Some(" ".into()))),
        _ => None,
    };
    if let Some((k, code, text)) = named { return Ok((k.into(), k.into(), code, text.map(Into::into))); }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<i64>().ok()).filter(|n| (1..=12).contains(n)) {
        return Ok((name.into(), name.into(), 111 + n, None));
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        let upper = c.to_ascii_uppercase();
        let code = if c.is_ascii_alphabetic() { format!("Key{upper}") } else if c.is_ascii_digit() { format!("Digit{c}") } else { String::new() };
        let key_code = if c.is_ascii_alphanumeric() { upper as i64 } else { 0 };
        return Ok((c.to_string(), code, key_code, Some(c.to_string())));
    }
    anyhow::bail!("Touche inconnue : {name}")
}

const MODIFIERS: [(&str, i64, i64); 4] = [("Alt", 1, 18), ("Control", 2, 17), ("Meta", 4, 91), ("Shift", 8, 16)];

impl Browser {
    fn page(&self) -> &Page { &self.pages[self.active] }

    async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.cdp.call(method, params, None, self.timeout).await
    }

    async fn page_call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.cdp.call(method, params, Some(&self.page().session), self.timeout).await
    }

    async fn eval(&self, expression: &str) -> anyhow::Result<Value> {
        let r = self.page_call("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true, "awaitPromise": true })).await?;
        if let Some(e) = r.get("exceptionDetails") {
            let text = e["exception"]["description"].as_str().or(e["text"].as_str()).unwrap_or("erreur JavaScript");
            anyhow::bail!("{text}");
        }
        Ok(r["result"]["value"].clone())
    }

    async fn attach(&self, target: &str) -> anyhow::Result<Page> {
        let r = self.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true })).await?;
        let session = r["sessionId"].as_str().unwrap_or_default().to_string();
        // Boîtes alert / confirm refusées automatiquement (événement Page.javascriptDialogOpening).
        let _ = self.cdp.call("Page.enable", json!({}), Some(&session), self.timeout).await;
        Ok(Page { target: target.into(), session })
    }

    /// Pages ouvertes (y compris les fenêtres ouvertes par la page), dans l'ordre d'ouverture.
    async fn sync_pages(&mut self) -> anyhow::Result<Vec<Value>> {
        let targets = self.call("Target.getTargets", json!({})).await?;
        let pages: Vec<Value> = targets["targetInfos"].as_array().cloned().unwrap_or_default().into_iter().filter(|t| t["type"] == "page").collect();
        let ids: Vec<String> = pages.iter().map(|t| t["targetId"].as_str().unwrap_or_default().to_string()).collect();
        let active = self.pages.get(self.active).map(|p| p.target.clone());
        self.pages.retain(|p| ids.contains(&p.target));
        for id in &ids {
            if !self.pages.iter().any(|p| &p.target == id) {
                let page = self.attach(id).await?;
                self.pages.push(page);
            }
        }
        self.active = active.and_then(|a| self.pages.iter().position(|p| p.target == a)).unwrap_or(0);
        Ok(pages)
    }

    async fn new_page(&mut self) -> anyhow::Result<()> {
        let r = self.call("Target.createTarget", json!({ "url": "about:blank" })).await?;
        let page = self.attach(r["targetId"].as_str().unwrap_or_default()).await?;
        self.pages.push(page);
        self.active = self.pages.len() - 1;
        Ok(())
    }

    /// Document chargé (DOMContentLoaded) dans le délai.
    async fn wait_loaded(&self) {
        let deadline = Instant::now() + self.timeout;
        tokio::time::sleep(Duration::from_millis(50)).await;
        while Instant::now() < deadline {
            match self.eval("document.readyState").await {
                Ok(v) if v != "loading" => return,
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    async fn snapshot(&self) -> anyhow::Result<String> {
        let elements: Vec<SnapElement> = serde_json::from_value(self.eval(COLLECT_ELEMENTS).await?).unwrap_or_default();
        let url = self.eval("location.href").await?.as_str().unwrap_or_default().to_string();
        let title = self.eval("document.title").await.ok().and_then(|t| t.as_str().map(str::to_string)).unwrap_or_default();
        Ok(format_snapshot(&url, &title, &elements))
    }

    /// Attend l'élément (présent et visible) ; renvoie le centre de sa boîte.
    async fn locate(&self, r: &str) -> anyhow::Result<(f64, f64)> {
        let sel = serde_json::to_string(&selector(r))?;
        let script = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return null; el.scrollIntoView({{ block: 'center', inline: 'center' }}); const b = el.getBoundingClientRect(); const s = getComputedStyle(el); if (!b.width || !b.height || s.visibility === 'hidden') return {{ hidden: true }}; return {{ x: b.left + b.width / 2, y: b.top + b.height / 2 }}; }})()"
        );
        let deadline = Instant::now() + self.timeout;
        loop {
            let v = self.eval(&script).await?;
            if let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) { return Ok((x, y)); }
            if Instant::now() >= deadline {
                anyhow::bail!("Élément [{r}] {} : refaites un snapshot (web_apercu).", if v.is_null() { "introuvable" } else { "invisible" });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn key(&self, kind: &str, def: &(String, String, i64, Option<String>), modifiers: i64) -> anyhow::Result<()> {
        let (key, code, key_code, text) = def;
        let mut params = json!({ "type": kind, "key": key, "code": code, "windowsVirtualKeyCode": key_code, "nativeVirtualKeyCode": key_code, "modifiers": modifiers });
        if kind == "keyDown" {
            match text.as_ref().filter(|_| modifiers & 0b0110 == 0) {
                Some(t) => { params["text"] = json!(t); params["unmodifiedText"] = json!(t); }
                None => params["type"] = json!("rawKeyDown"),
            }
        }
        self.page_call("Input.dispatchKeyEvent", params).await?;
        Ok(())
    }

    /// « Control+Shift+A », « Enter »…
    async fn press(&self, combo: &str) -> anyhow::Result<()> {
        let parts: Vec<&str> = if combo == "+" { vec!["+"] } else { combo.split('+').collect() };
        let (last, mods) = parts.split_last().ok_or_else(|| anyhow::anyhow!("Touche vide"))?;
        let mut modifiers = 0;
        let mut held = vec![];
        for m in mods {
            let (name, bit, code) = MODIFIERS.iter().find(|(n, ..)| n.eq_ignore_ascii_case(m) || (*m == "Ctrl" && *n == "Control"))
                .ok_or_else(|| anyhow::anyhow!("Modificateur inconnu : {m}"))?;
            modifiers |= bit;
            let def = (name.to_string(), format!("{name}Left"), *code, None);
            self.key("keyDown", &def, modifiers).await?;
            held.push(def);
        }
        let def = key_def(last)?;
        self.key("keyDown", &def, modifiers).await?;
        self.key("keyUp", &def, modifiers).await?;
        for def in held.iter().rev() { self.key("keyUp", def, modifiers).await?; }
        Ok(())
    }
}

impl WebHub {
    pub fn new(config: WebConfigFn, user_agent: &str) -> Self {
        WebHub { config, user_agent: user_agent.into(), browser: Mutex::new(None) }
    }

    async fn launch(&self) -> anyhow::Result<Browser> {
        let cfg = (self.config)();
        let channel = cfg.browser.as_str();
        let exe = candidates(channel).into_iter().find(|p| p.is_file()).ok_or_else(|| {
            anyhow::anyhow!("Impossible de lancer le navigateur « {channel} » : exécutable introuvable. Vérifiez que Microsoft Edge (ou Chrome) est installé, ou changez web.browser dans config.yaml.")
        })?;
        let profile = std::env::temp_dir().join(format!("ocekour-web-{}", uuid::Uuid::new_v4()));
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.args([
            "--remote-debugging-port=0".to_string(),
            format!("--user-data-dir={}", profile.display()),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            "--disable-sync".into(),
            "--disable-background-networking".into(),
            "--disable-features=Translate,MediaRouter".into(),
            format!("--user-agent={}", self.user_agent),
        ]);
        if cfg.headless { cmd.arg("--headless=new"); }
        if cfg.tls.insecure { cmd.arg("--ignore-certificate-errors"); }
        if running_as_root() { cmd.arg("--no-sandbox"); }
        cmd.arg("about:blank");
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("Impossible de lancer le navigateur « {channel} » : {e}. Vérifiez que Microsoft Edge (ou Chrome) est installé, ou changez web.browser dans config.yaml."))?;
        let stderr = child.stderr.take().unwrap();
        let mut lines = BufReader::new(stderr).lines();
        let ws = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(url) = line.strip_prefix("DevTools listening on ") { return Some(url.trim().to_string()); }
            }
            None
        })
        .await
        .ok()
        .flatten()
        .ok_or_else(|| anyhow::anyhow!("Impossible de lancer le navigateur « {channel} » : pas de réponse du protocole DevTools."))?;
        // Lecture du reste de stderr (sinon le navigateur peut se bloquer en écriture).
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

        let cdp_slot: Arc<parking_lot::Mutex<Option<std::sync::Weak<Cdp>>>> = Arc::default();
        let on_event: EventHandler = {
            let slot = cdp_slot.clone();
            Arc::new(move |method: &str, _params: &Value, session: Option<&str>| {
                if method != "Page.javascriptDialogOpening" { return; }
                let (Some(cdp), Some(session)) = (slot.lock().as_ref().and_then(|w| w.upgrade()), session.map(str::to_string)) else { return };
                tokio::spawn(async move {
                    let _ = cdp.call("Page.handleJavaScriptDialog", json!({ "accept": false }), Some(&session), Duration::from_secs(5)).await;
                });
            })
        };
        let cdp = Cdp::connect(&ws, on_event).await?;
        *cdp_slot.lock() = Some(Arc::downgrade(&cdp));
        let mut browser = Browser { child, cdp, pages: vec![], active: 0, profile, timeout: Duration::from_millis(cfg.timeout_ms) };
        browser.sync_pages().await?;
        if browser.pages.is_empty() { browser.new_page().await?; }
        log::info!("[web] navigateur lancé ({channel}, {})", if cfg.headless { "invisible" } else { "visible" });
        Ok(browser)
    }

    /// Navigateur prêt (lancé au besoin, relancé s'il a été fermé), avec une page active.
    async fn ready<'a>(&self, guard: &'a mut Option<Browser>) -> anyhow::Result<&'a mut Browser> {
        if guard.as_ref().is_some_and(|b| b.cdp.is_closed()) {
            if let Some(b) = guard.take() { shutdown(b).await; }
        }
        if guard.is_none() { *guard = Some(self.launch().await?); }
        let b = guard.as_mut().unwrap();
        b.sync_pages().await?;
        if b.pages.is_empty() { b.new_page().await?; }
        Ok(b)
    }

    /// Navigue (nouvel onglet si demandé) puis renvoie le snapshot.
    pub async fn goto(&self, url: &str, new_tab: bool) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        if new_tab { b.new_page().await?; }
        let _ = b.page_call("Page.bringToFront", json!({})).await;
        let r = b.page_call("Page.navigate", json!({ "url": url })).await?;
        if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) { anyhow::bail!("page.goto : {e} ({url})"); }
        b.wait_loaded().await;
        b.snapshot().await
    }

    /// Snapshot d'accessibilité de la page active (marque les refs dans la page).
    pub async fn snapshot(&self) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        self.ready(&mut guard).await?.snapshot().await
    }

    pub async fn click(&self, r: &str) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        let (x, y) = b.locate(r).await?;
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            b.page_call("Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": if kind == "mouseMoved" { 0 } else { 1 } })).await?;
        }
        b.wait_loaded().await;
        b.snapshot().await
    }

    pub async fn type_text(&self, r: &str, text: &str, submit: bool) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        b.locate(r).await?;
        let sel = serde_json::to_string(&selector(r))?;
        b.eval(&format!(
            "(() => {{ const el = document.querySelector({sel}); el.focus(); if (typeof el.select === 'function') el.select(); else {{ const range = document.createRange(); range.selectNodeContents(el); const s = getSelection(); s.removeAllRanges(); s.addRange(range); }} }})()"
        )).await?;
        if text.is_empty() { b.press("Backspace").await?; } else { b.page_call("Input.insertText", json!({ "text": text })).await?; }
        if submit {
            b.press("Enter").await?;
            b.wait_loaded().await;
        }
        b.snapshot().await
    }

    pub async fn select(&self, r: &str, values: &[String]) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        b.locate(r).await?;
        let (sel, wanted) = (serde_json::to_string(&selector(r))?, serde_json::to_string(values)?);
        let found = b.eval(&format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el || el.tagName !== 'SELECT') return -1; const wanted = {wanted}; const opts = Array.from(el.options); const hits = opts.filter((o) => wanted.includes(o.value) || wanted.includes(o.label.trim())); if (!hits.length) return 0; if (el.multiple) opts.forEach((o) => (o.selected = hits.includes(o))); else el.selectedIndex = opts.indexOf(hits[0]); el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }})); return hits.length; }})()"
        )).await?;
        match found.as_i64() {
            Some(-1) => anyhow::bail!("L'élément [{r}] n'est pas une liste <select>."),
            Some(0) => anyhow::bail!("Aucune option ne correspond à {}.", values.join(", ")),
            _ => b.snapshot().await,
        }
    }

    pub async fn press(&self, key: &str) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        b.press(key).await?;
        b.wait_loaded().await;
        b.snapshot().await
    }

    /// Attend un texte visible, ou un délai (ms), puis renvoie le snapshot.
    pub async fn wait_for(&self, text: Option<&str>, ms: Option<u64>) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        if let Some(t) = text.filter(|t| !t.is_empty()) {
            let needle = serde_json::to_string(&t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase())?;
            let script = format!("(document.body?.innerText ?? '').replace(/\\s+/g, ' ').toLowerCase().includes({needle})");
            let deadline = Instant::now() + b.timeout;
            while b.eval(&script).await? != json!(true) {
                if Instant::now() >= deadline { anyhow::bail!("Texte « {t} » non visible après {} ms.", b.timeout.as_millis()); }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        if let Some(ms) = ms { tokio::time::sleep(Duration::from_millis(ms.min(60_000))).await; }
        b.snapshot().await
    }

    /// Texte visible de la page active.
    pub async fn text(&self) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        Ok(b.eval("document.body?.innerText ?? ''").await?.as_str().unwrap_or_default().to_string())
    }

    /// Capture PNG en base64.
    pub async fn screenshot(&self, full_page: bool) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let b = self.ready(&mut guard).await?;
        let _ = b.page_call("Page.bringToFront", json!({})).await;
        let mut params = json!({ "format": "png" });
        if full_page {
            let m = b.page_call("Page.getLayoutMetrics", json!({})).await?;
            let size = if m["cssContentSize"].is_object() { &m["cssContentSize"] } else { &m["contentSize"] };
            params["captureBeyondViewport"] = json!(true);
            params["clip"] = json!({ "x": 0, "y": 0, "width": size["width"], "height": size["height"], "scale": 1 });
        }
        let r = b.page_call("Page.captureScreenshot", params).await?;
        Ok(r["data"].as_str().unwrap_or_default().to_string())
    }

    pub async fn tabs(&self) -> anyhow::Result<Vec<TabInfo>> {
        let mut guard = self.browser.lock().await;
        let Some(b) = guard.as_mut().filter(|b| !b.cdp.is_closed()) else { return Ok(vec![]) };
        let infos = b.sync_pages().await?;
        Ok(b.pages.iter().enumerate().map(|(index, p)| {
            let info = infos.iter().find(|t| t["targetId"] == p.target.as_str());
            TabInfo {
                index,
                actif: index == b.active,
                titre: info.and_then(|i| i["title"].as_str()).unwrap_or_default().into(),
                url: info.and_then(|i| i["url"].as_str()).unwrap_or_default().into(),
            }
        }).collect())
    }

    /// Ferme un onglet (index) ou tout le navigateur si `index` absent.
    pub async fn close(&self, index: Option<usize>) -> anyhow::Result<String> {
        let mut guard = self.browser.lock().await;
        let Some(index) = index.filter(|_| guard.is_some()) else {
            if let Some(b) = guard.take() { shutdown(b).await; }
            return Ok("Navigateur fermé.".into());
        };
        let b = guard.as_mut().unwrap();
        b.sync_pages().await?;
        let Some(page) = b.pages.get(index).cloned() else { anyhow::bail!("Aucun onglet à l'index {index}.") };
        let active = b.pages[b.active].target.clone();
        b.call("Target.closeTarget", json!({ "targetId": page.target })).await?;
        b.pages.remove(index);
        b.active = b.pages.iter().position(|p| p.target == active).unwrap_or(0);
        let remaining = b.pages.len();
        if remaining == 0 {
            if let Some(b) = guard.take() { shutdown(b).await; }
            return Ok("Dernier onglet fermé : navigateur fermé.".into());
        }
        Ok(format!("Onglet {index} fermé ({remaining} restant{}).", if remaining > 1 { "s" } else { "" }))
    }

    /// Ferme tout (arrêt de l'application).
    pub async fn dispose(&self) {
        if let Some(b) = self.browser.lock().await.take() { shutdown(b).await; }
    }

    pub async fn is_running(&self) -> bool { self.browser.lock().await.as_ref().is_some_and(|b| !b.cdp.is_closed()) }
}

async fn shutdown(mut b: Browser) {
    let _ = tokio::time::timeout(Duration::from_secs(2), b.cdp.call("Browser.close", json!({}), None, Duration::from_secs(2))).await;
    b.cdp.close();
    if tokio::time::timeout(Duration::from_secs(3), b.child.wait()).await.is_err() { let _ = b.child.kill().await; }
    let _ = std::fs::remove_dir_all(&b.profile);
}
