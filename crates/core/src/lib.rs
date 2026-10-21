//! Cœur d'Ocekour : tout ce que fait l'application, sans aucune interface. L'interface
//! (Tauri) et le mode sans interface (MCP, notifications) ne sont que des surcouches.

pub mod config;
pub mod error;
pub mod kafka;
pub mod logging;
pub mod logs;
pub mod net;
pub mod rancher;
pub mod s3;
pub mod schema;
pub mod secrets;
pub mod text;
pub mod yaml;
pub mod streams;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
