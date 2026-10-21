// Clé d'activation (docs/cles-activation.md) :
// - OCEKOUR_RELEASE=1 (exe diffusé, CI) : clés de production seulement ; exige au moins une
//   clé publique de production et la clé d'obfuscation LICENCE_OBFUSCATION (32 octets base64).
// - sinon (développement, tests) : clés de production et de développement acceptées.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(ocekour_release)");
    println!("cargo:rerun-if-env-changed=OCEKOUR_RELEASE");
    println!("cargo:rerun-if-env-changed=LICENCE_OBFUSCATION");
    if std::env::var("OCEKOUR_RELEASE").as_deref() == Ok("1") {
        let key = std::env::var("LICENCE_OBFUSCATION").unwrap_or_default();
        let valid = key.len() == 44 && key.ends_with('=');
        let keys = std::fs::read_to_string("src/licence/keys.rs").unwrap_or_default();
        let marker = "// --- clés de production (ne pas retirer ce marqueur) ---";
        let has_prod = keys.split(marker).nth(1).is_some_and(|rest| rest.split("];").next().unwrap_or("").contains("BEGIN PUBLIC KEY"));
        let mut problems = vec![];
        if !has_prod { problems.push("aucune clé publique de production : lancez `ocekour-headless cle init`"); }
        if !valid { problems.push("secret LICENCE_OBFUSCATION absent ou invalide (32 octets en base64)"); }
        if !problems.is_empty() {
            panic!("Build de diffusion impossible :\n- {}\nVoir docs/cles-activation.md, « Mise en place ».", problems.join("\n- "));
        }
        println!("cargo:rustc-cfg=ocekour_release");
    }
}
