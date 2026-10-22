//! Administration des clés d'activation (voir docs/cles-activation.md).
//!
//!   cle init --kid prod1 --dossier <dossier privé>
//!   cle generer --kid prod1 --prive <fichier.pem> --label "Équipe X" --mode none|optional|required
//!        [--matomo-url URL --site-id N] [--dim-version N] [--dim-cle N] [--matomo-insecure]
//!        [--preconfig preconfiguration.yaml]
//!   cle lire <clé>
//!
//! La clé d'obfuscation de production se passe par la variable LICENCE_OBFUSCATION.

use ocekour_core::licence::format::{Keyring, decode, encode, generate_obfuscation_key, generate_signing_keys, new_licence_id};
use ocekour_core::licence::keys::{DEV_OBFUSCATION_KEY, DEV_PUBLIC_KEYS, PRODUCTION_PUBLIC_KEYS};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MARKER: &str = "    // --- clés de production (ne pas retirer ce marqueur) ---";

struct Opts {
    values: HashMap<String, String>,
    flags: Vec<String>,
    positionals: Vec<String>,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts { values: HashMap::new(), flags: vec![], positionals: vec![] };
    let mut i = 0;
    while i < args.len() {
        if let Some(name) = args[i].strip_prefix("--") {
            if name == "matomo-insecure" {
                o.flags.push(name.into());
            } else if let Some((k, v)) = name.split_once('=') {
                o.values.insert(k.into(), v.into());
            } else if i + 1 < args.len() {
                o.values.insert(name.into(), args[i + 1].clone());
                i += 1;
            }
        } else {
            o.positionals.push(args[i].clone());
        }
        i += 1;
    }
    o
}

fn obfuscation_for(kid: &str) -> Result<String, String> {
    if kid == "dev" { return Ok(DEV_OBFUSCATION_KEY.into()); }
    std::env::var("LICENCE_OBFUSCATION").ok().filter(|k| !k.is_empty()).ok_or_else(|| "variable LICENCE_OBFUSCATION absente.".into())
}

fn int(o: &Opts, name: &str) -> Result<Option<i64>, String> {
    match o.values.get(name) {
        None => Ok(None),
        Some(v) => v.parse::<i64>().ok().filter(|n| *n > 0).map(Some).ok_or_else(|| format!("--{name} : entier positif attendu")),
    }
}

fn repo_root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap_or_else(|_| PathBuf::from(".")) }

/// Point d'entrée ; renvoie le code de sortie.
pub fn main(args: &[String]) -> i32 {
    let command = args.first().map(String::as_str).unwrap_or_default();
    let o = parse(args.get(1..).unwrap_or_default());
    let result = match command {
        "init" => init(&o),
        "generer" => generer(&o),
        "lire" => lire(&o),
        _ => {
            println!("Usage : ocekour-headless cle init | generer | lire  (voir docs/cles-activation.md)");
            return if command.is_empty() { 0 } else { 1 };
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Erreur : {e}");
            1
        }
    }
}

fn init(o: &Opts) -> Result<(), String> {
    let kid = o.values.get("kid").ok_or("--kid obligatoire (ex. prod1).")?;
    if kid == "dev" || kid.is_empty() || kid.len() > 20 || !kid.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("kid invalide : {kid}"));
    }
    if PRODUCTION_PUBLIC_KEYS.iter().any(|(k, _)| k == kid) { return Err(format!("la clé « {kid} » existe déjà.")); }
    let root = repo_root();
    let dir = std::path::absolute(o.values.get("dossier").map(String::as_str).unwrap_or(".")).map_err(|e| e.to_string())?;
    if dir.starts_with(&root) { return Err("le dossier privé doit être HORS du dépôt (--dossier).".into()); }
    let (private, public) = generate_signing_keys();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let private_file = dir.join(format!("ocekour-{kid}.pem"));
    if private_file.exists() { return Err(format!("{} existe déjà.", private_file.display())); }
    std::fs::write(&private_file, private).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&private_file, std::fs::Permissions::from_mode(0o600));
    }
    let keys_file = root.join("crates/core/src/licence/keys.rs");
    let source = std::fs::read_to_string(&keys_file).map_err(|e| e.to_string())?;
    if !source.contains(MARKER) { return Err(format!("marqueur introuvable dans {}", keys_file.display())); }
    std::fs::write(&keys_file, source.replace(MARKER, &format!("{MARKER}\n    (\"{kid}\", \"{}\"),", public.trim()))).map_err(|e| e.to_string())?;
    println!("Clé privée de signature : {}", private_file.display());
    println!("  → à garder hors du dépôt (coffre de mots de passe), jamais diffusée.");
    println!("Clé publique ajoutée dans crates/core/src/licence/keys.rs : à commiter.");
    if std::env::var("LICENCE_OBFUSCATION").is_err() {
        println!("\nClé d’obfuscation (secret GitHub LICENCE_OBFUSCATION, à garder aussi) :\n  {}", generate_obfuscation_key());
    }
    Ok(())
}

fn generer(o: &Opts) -> Result<(), String> {
    let kid = o.values.get("kid").ok_or("--kid obligatoire.")?;
    let private_file = match o.values.get("prive") {
        Some(p) => PathBuf::from(p),
        None if kid == "dev" => repo_root().join("scripts/cles-dev/dev.pem"),
        None => return Err("--prive <fichier.pem> obligatoire.".into()),
    };
    let mode = o.values.get("mode").ok_or("--mode none|optional|required obligatoire.")?;
    let label_mode = match mode.as_str() { "none" => "sans suivi", "optional" => "suivi si possible", "required" => "suivi obligatoire", _ => return Err("--mode : none, optional ou required".into()) };
    let label = o.values.get("label").ok_or("--label obligatoire (à qui est remise la clé).")?;
    let mut payload = json!({ "v": 1, "id": new_licence_id(), "label": label, "issuedAt": ocekour_core::text::now_iso()[..10].to_string(), "mode": mode });
    if mode != "none" {
        let mut dimensions = Map::new();
        if let Some(v) = int(o, "dim-version")? { dimensions.insert("version".into(), json!(v)); }
        if let Some(v) = int(o, "dim-cle")? { dimensions.insert("key".into(), json!(v)); }
        payload["matomo"] = json!({
            "url": o.values.get("matomo-url").ok_or("--matomo-url obligatoire avec suivi.")?,
            "siteId": int(o, "site-id")?.ok_or("--site-id obligatoire avec suivi.")?,
            "insecure": o.flags.iter().any(|f| f == "matomo-insecure"),
            "dimensions": dimensions,
        });
    }
    if let Some(file) = o.values.get("preconfig") {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{file} : {e}"))?;
        let raw = ocekour_core::yaml::parse(&text).map_err(|e| format!("{file} : {}", e.message))?.to_json();
        let valid = ocekour_core::licence::preset::validate(Some(&raw)).map_err(|e| format!("{file} → {e}"))?;
        // Services : seulement url et tls (activation et écriture : choix de chacun).
        let services: Map<String, Value> = valid.services.iter().map(|(id, s)| {
            let mut v = json!({ "url": s["url"] });
            if s["tls"]["insecure"] == true || s["tls"]["caFile"].is_string() { v["tls"] = s["tls"].clone(); }
            (id.clone(), v)
        }).collect();
        let mut preset = Map::new();
        let connections: Vec<Value> = valid.connections.iter().map(|c| Value::Object(ocekour_core::connections::normalize(c))).collect();
        if !connections.is_empty() { preset.insert("connections".into(), json!(connections)); }
        if !valid.profiles.is_empty() { preset.insert("profiles".into(), serde_json::to_value(&valid.profiles).map_err(|e| e.to_string())?); }
        if !services.is_empty() { preset.insert("services".into(), Value::Object(services.clone())); }
        eprintln!(
            "Préconfiguration : {} connexion(s) ({}), {} profil(s), services : {}.",
            valid.connections.len(),
            if valid.connections.is_empty() { "—".into() } else { valid.connections.iter().map(|c| c.name().to_string()).collect::<Vec<_>>().join(", ") },
            valid.profiles.len(),
            if services.is_empty() { "—".into() } else { services.keys().cloned().collect::<Vec<_>>().join(", ") },
        );
        payload["preset"] = Value::Object(preset);
    }
    let private = std::fs::read_to_string(&private_file).map_err(|e| format!("{} : {e}", private_file.display()))?;
    let key = encode(&payload, kid, &private, &obfuscation_for(kid)?).map_err(|e| e.to_string())?;
    eprintln!("Clé {} · {label} · {label_mode} :", payload["id"].as_str().unwrap_or_default());
    println!("{key}");
    eprintln!("\nÀ noter dans votre registre des clés émises (id, libellé, type, date, destinataire).");
    Ok(())
}

fn lire(o: &Opts) -> Result<(), String> {
    let text = o.positionals.join("");
    if text.is_empty() { return Err("clé attendue.".into()); }
    let mut obfuscation_keys = vec![DEV_OBFUSCATION_KEY.to_string()];
    if let Ok(k) = std::env::var("LICENCE_OBFUSCATION") { if !k.is_empty() { obfuscation_keys.push(k); } }
    let public_keys = DEV_PUBLIC_KEYS.iter().chain(PRODUCTION_PUBLIC_KEYS).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let (payload, _) = decode(&text, &Keyring { public_keys, obfuscation_keys }).map_err(|e| e.0)?;
    println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
    Ok(())
}
