//! WebView2 (Windows) : runtime installé sur le poste, ou runtime « Fixed Version » posé à côté
//! de l'exe, pour les postes sans droits d'installation ni accès Internet (Windows 10 n'a pas
//! toujours WebView2). Vérifié avant d'ouvrir la fenêtre : message clair plutôt qu'un échec brut.

use std::path::{Path, PathBuf};

/// Variable lue par WebView2 : dossier du runtime à utiliser au lieu de celui installé.
pub const BROWSER_FOLDER_VAR: &str = "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER";

/// Runtime fourni avec l'exe : `WebView2\` ou `Microsoft.WebView2.FixedVersionRuntime.<version>.x64\`
/// (nom du dossier livré par Microsoft ; le plus récent s'il y en a plusieurs).
pub fn bundled_runtime(exe_dir: &Path) -> Option<PathBuf> {
    let has_browser = |dir: &Path| dir.join("msedgewebview2.exe").is_file();
    let named = exe_dir.join("WebView2");
    if has_browser(&named) { return Some(named); }
    let mut found: Vec<PathBuf> = std::fs::read_dir(exe_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Microsoft.WebView2.FixedVersionRuntime.")))
        .filter(|p| has_browser(p))
        .collect();
    // Par version (140 > 99), pas par ordre alphabétique.
    let version = |p: &PathBuf| -> Vec<u64> {
        p.file_name().and_then(|n| n.to_str()).unwrap_or("").split(['.', '-']).filter_map(|x| x.parse().ok()).collect()
    };
    found.sort_by_key(version);
    found.pop()
}

pub const MISSING: &str = "Ocekour a besoin du composant Microsoft Edge WebView2, absent de ce poste.\n\n\
Sans installation ni accès Internet : copiez le runtime WebView2 « Fixed Version » à côté de \
l'exe d'Ocekour, dans un dossier nommé WebView2 (il doit contenir msedgewebview2.exe). \
Le zip d'Ocekour le contient déjà.\n\n\
Autre solution : faire installer « Microsoft Edge WebView2 Runtime » par le service informatique.\n\n\
ocekour-headless.exe (sonde, serveur MCP, notifications) n'en a pas besoin.";

/// Prépare WebView2 : runtime livré avec l'exe s'il y en a un, puis vérifie qu'un runtime est
/// utilisable. À appeler avant de lancer d'autres fils (modifie l'environnement du processus).
#[cfg(windows)]
pub fn prepare() -> Result<String, &'static str> {
    if std::env::var_os(BROWSER_FOLDER_VAR).is_none() {
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
        if let Some(dir) = exe_dir.as_deref().and_then(bundled_runtime) {
            grant_sandbox_access(&dir);
            // SAFETY : appelé au démarrage, avant la création de tout autre fil.
            unsafe { std::env::set_var(BROWSER_FOLDER_VAR, &dir) };
        }
    }
    installed_version().ok_or(MISSING)
}

/// Version du runtime que WebView2 utilisera (celui de la variable, sinon celui installé).
#[cfg(windows)]
fn installed_version() -> Option<String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
    let mut version = windows_core::PWSTR::null();
    // SAFETY : `version` reçoit une chaîne allouée par WebView2, libérée par `take_pwstr`.
    let ok = unsafe { GetAvailableCoreWebView2BrowserVersionString(windows_core::PCWSTR::null(), &mut version) }.is_ok();
    if !ok || version.is_null() { return None; }
    Some(webview2_com::take_pwstr(version)).filter(|v| !v.is_empty())
}

/// Windows 10 : le bac à sable de WebView2 doit pouvoir lire un runtime Fixed Version placé hors
/// de Program Files (« ALL APPLICATION PACKAGES » et « ALL RESTRICTED APPLICATION PACKAGES »).
/// Fait une fois (marqueur dans le dossier) ; sans droits sur le dossier, WebView2 le signalera.
#[cfg(windows)]
fn grant_sandbox_access(dir: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let marker = dir.join(".ocekour-acl");
    if marker.exists() { return; }
    let mut ok = true;
    for sid in ["*S-1-15-2-2", "*S-1-15-2-1"] {
        let status = std::process::Command::new("icacls")
            .arg(dir)
            .args(["/grant", &format!("{sid}:(OI)(CI)(RX)"), "/T", "/Q"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        ok &= status.is_ok_and(|s| s.success());
    }
    if ok { let _ = std::fs::write(marker, ""); }
}

/// Boîte de message native (avant toute fenêtre).
#[cfg(windows)]
pub fn alert(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Ocekour"));
    // SAFETY : chaînes terminées par 0, vivantes pendant l'appel.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("msedgewebview2.exe"), "").unwrap();
    }

    #[test]
    fn runtime_a_cote_de_l_exe() {
        let exe = tempfile::tempdir().unwrap();
        assert_eq!(bundled_runtime(exe.path()), None);
        // Dossier au nom de Microsoft, sans msedgewebview2.exe : ignoré.
        std::fs::create_dir(exe.path().join("Microsoft.WebView2.FixedVersionRuntime.140.0.1.x64")).unwrap();
        assert_eq!(bundled_runtime(exe.path()), None);
        runtime(&exe.path().join("Microsoft.WebView2.FixedVersionRuntime.99.0.2.x64"));
        runtime(&exe.path().join("Microsoft.WebView2.FixedVersionRuntime.141.0.3.x64"));
        assert_eq!(bundled_runtime(exe.path()), Some(exe.path().join("Microsoft.WebView2.FixedVersionRuntime.141.0.3.x64")));
        // `WebView2\` est prioritaire.
        runtime(&exe.path().join("WebView2"));
        assert_eq!(bundled_runtime(exe.path()), Some(exe.path().join("WebView2")));
    }
}
