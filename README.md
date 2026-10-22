# Ocekour 2

Visionneuse de logs locale pour Windows : logs Rancher, topics Kafka, explorateur S3, serveur
MCP pour les agents (GitLab, Jira, Confluence, Rocket.Chat, web, bases de données en lecture
seule), notifications et contexte IA.

Réécriture de la version Electron/Angular en **Rust + Tauri 2**, interface en **Svelte 5**.

- **Guide utilisateur** : [docs/guide/](docs/guide/README.md) (historique dans le [CHANGELOG](docs/guide/CHANGELOG.md))
- Cadrage : [docs/cadrage.md](docs/cadrage.md) · Clés d'activation : [docs/cles-activation.md](docs/cles-activation.md)

## Architecture : un cœur, des surcouches

```
crates/core/      ocekour-core : TOUTE la logique, sans interface (bibliothèque)
                  config.yaml, secrets, licence, statistiques, Rancher/K8s, Kafka (protocole
                  natif + Avro), S3 (SigV4), flux de logs, MCP (stdio + HTTP) et ses outils,
                  services, navigateur (DevTools), bases de données, notifications, contexte IA,
                  api.rs : les canaux de l'interface (mêmes noms que la version Electron)
crates/headless/  ocekour-headless : exe sans interface (--mcp, --mcp-http, --notify, clés)
src-tauri/        ocekour : la fenêtre. Une seule commande, `call(canal, arguments)`, qui
                  délègue à ocekour_core::api ; le cœur pousse ses événements à la fenêtre
ui/               interface Svelte 5 + Tailwind 4 (Vite). `window.ocekour` = pont vers `call`
shared/ (ui/)     contrat typé interface ↔ cœur (api.ts) et schémas de config.yaml
```

L'interface n'est qu'une surcouche : tout ce qu'elle fait passe par `ocekour_core::api::call`,
que l'on peut appeler sans fenêtre. Le mode sans interface ne charge ni Tauri ni WebView2 :

| Commande                                      | Effet                                                     |
| --------------------------------------------- | --------------------------------------------------------- |
| `ocekour.exe` ou `ocekour-headless.exe --mcp` | serveur MCP sur stdio (lancé par le client)               |
| `… --mcp-http [--mcp-port=N]`                 | serveur MCP HTTP sur 127.0.0.1, avec jeton                |
| `… --notify`                                  | notifications GitLab / Jira / Confluence → Rocket.Chat    |
| `ocekour-headless cle init\|generer\|lire`    | clés d'activation (administration)                        |
| `ocekour-headless fake-rancher [--port N]`    | faux Rancher HTTPS pour essayer l'interface sans cluster  |

Option commune : `--data-dir=<dossier>` (sinon `OCEKOUR_DATA_DIR`, sinon `%APPDATA%\Ocekour`).
Les fichiers (`config.yaml`, `secrets.json`, `secret.key`, `actions.ndjson`, `contexte/`…) et
les clés d'activation sont ceux de la 1.x : la migration n'a rien à refaire.

## Gains par rapport à la version Electron

- Exe d'une vingtaine de Mo (contre ~330 Mo décompressés au premier lancement) ; démarrage
  immédiat, WebView2 du système.
- Bundle initial de l'interface ~210 Ko (CodeMirror, explorateur S3 et Paramètres chargés à la
  demande).
- Kafka, S3, Oracle, SQL Server, PostgreSQL, navigateur : clients natifs Rust, plus de
  `node_modules` embarqués (kafkajs, AWS SDK, Playwright, oracledb…).

## Commandes

Prérequis : Rust stable, Node ≥ 22. Sous Linux, WebKitGTK 4.1 (`libwebkit2gtk-4.1-dev`).

| Commande                                         | Effet                                                   |
| ------------------------------------------------ | ------------------------------------------------------- |
| `npm --prefix ui ci`                             | Dépendances de l'interface                              |
| `node ui/node_modules/@tauri-apps/cli/tauri.js dev` | Dev (depuis la racine) : Vite + fenêtre, rechargement du front |
| `node ui/node_modules/@tauri-apps/cli/tauri.js build` | `target/release/ocekour.exe` + installateur NSIS |
| `cargo build --release -p ocekour-headless`      | Exe sans interface seul                                 |
| `cargo test --workspace`                         | Tests Rust (cœur, sans interface, IPC de la fenêtre)    |
| `npm --prefix ui test` / `npm --prefix ui run check` | Tests de l'interface (vitest) / svelte-check        |

Les tests utilisent de vrais serveurs quand ils sont présents (sinon ils sont ignorés) :

| Variable                   | Serveur                                                         |
| -------------------------- | --------------------------------------------------------------- |
| `KAFKA_TEST_BROKER`        | Kafka 4 (`scripts/kafka-test-broker.sh`, `localhost:19092`)      |
| `KAFKA_TEST_LEGACY_BROKER` | Kafka 3.0 (`scripts/kafka-legacy-broker.sh`, `localhost:19192`)  |
| `S3_TEST_URL`              | S3 simulé moto (`scripts/s3-test-server.sh`)                    |
| `BDD_TEST_PG`              | PostgreSQL (`scripts/pg-test-fixture.sql`)                      |
| `OCEKOUR_BROWSER_PATH`     | Chrome/Chromium/Edge pour les outils `web_*`                    |

La CI (`.github/workflows/build.yml`) lance tous les tests sous Linux avec ces serveurs, puis
construit sous Windows `Ocekour-<version>.exe`, `ocekour-headless.exe` et l'installateur, avec
les clés d'activation de production (`OCEKOUR_RELEASE=1`, secret `LICENCE_OBFUSCATION`).

En développement, une clé : `cargo run -p ocekour-headless -- cle generer --kid dev --label Dev --mode none`.
