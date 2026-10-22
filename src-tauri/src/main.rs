// Pas de console sous Windows en production (la fenêtre suffit ; les modes sans interface
// se rattachent à la console du terminal qui les lance).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    std::process::exit(ocekour_app::main());
}
