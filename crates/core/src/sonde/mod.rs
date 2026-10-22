//! Sonde : le même exécutable, sans interface, sur une machine distante, expose en lecture
//! seule des sources de logs (fichiers, motifs) aux postes qui s'y connectent.

pub mod cli;
pub mod client;
pub mod config;
pub mod server;
pub mod service;
