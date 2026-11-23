fn main() {
    // Windows : pile du fil principal à 8 Mo au lieu de 1 Mo (le mode sans interface y exécute
    // ses tâches asynchrones ; une pile trop juste ferme le programme sans message).
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    } else if target.contains("windows-gnu") {
        println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608");
    }
}
