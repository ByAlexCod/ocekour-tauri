//! Clés publiques acceptées par l'exe et clés révoquées. Voir docs/cles-activation.md.
//! Les clés PRIVÉES de production ne sont jamais dans ce dépôt.

/// Clés publiques de production, par kid (ajoutées par `ocekour-headless cle init`).
pub const PRODUCTION_PUBLIC_KEYS: &[(&str, &str)] = &[
    // --- clés de production (ne pas retirer ce marqueur) ---
    ("prod1", "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAfGRQH2jpxqwjWp4cornwH3m8ZQqDCSxyYPcjVlvE/DM=
-----END PUBLIC KEY-----"),
];

/// Clés de DÉVELOPPEMENT : la clé privée est volontairement dans le dépôt
/// (scripts/cles-dev/dev.pem). Acceptées uniquement par les builds de développement.
pub const DEV_PUBLIC_KEYS: &[(&str, &str)] = &[(
    "dev",
    "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEA6UyleJSTML525rGOixFu/WYwbtdd6TswFsTu6Y5EMHo=
-----END PUBLIC KEY-----",
)];

pub const DEV_OBFUSCATION_KEY: &str = "7reacn7viM9L/gGxRd+LW2Ycs9hIhCfreXI8gmEkNk0=";

/// Identifiants de clés révoquées (clé diffusée par erreur, départ d'une équipe…).
pub const REVOKED_KEY_IDS: &[&str] = &[];

/// Exe diffusé (`OCEKOUR_RELEASE=1` à la compilation) : clés de production seulement.
pub const RELEASE: bool = cfg!(ocekour_release);

/// Clé d'obfuscation de production (secret `LICENCE_OBFUSCATION` à la compilation).
pub const OBFUSCATION_KEY: Option<&str> = option_env!("LICENCE_OBFUSCATION");
