fn main() {
    // Windows : pile du fil principal (boucle de la fenêtre) à 8 Mo au lieu de 1 Mo. Tauri y
    // construit les futurs des commandes ; une pile trop juste fait fermer l'application sans
    // message (débordement de pile).
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    } else if target.contains("windows-gnu") {
        println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608");
    }
    tauri_build::build()
}
