//! Outils MCP : les mêmes actions que l'interface. Tant que l'application n'est pas activée,
//! seuls les outils de licence (et l'attente) répondent.

use super::logs::{ReadRequest, read_logs, workload_of};
use super::{Args, Content, Tool, fail, ok, tool};
use crate::app::App;
use crate::config::Connection;
use crate::logs::parser::Level;
use crate::logs::{ExportFormat, LogEntry, SearchOptions, export_content, share};
use crate::obj;
use crate::s3::S3Entry;
use crate::s3::service::{S3Op, S3Progress};
use crate::schema::{self as z, S};
use crate::streams::StreamOptions;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LEVELS: [&str; 7] = ["trace", "debug", "info", "warn", "error", "fatal", "unknown"];

fn log_input(max: S) -> Vec<(&'static str, S)> {
    vec![
        ("connexion", z::string().describe("Id de la connexion (voir connexions_lister)")),
        ("chemin", z::array(z::string().min(1)).min(1).max(4).describe("Rancher : [cluster, namespace, pod, conteneur] ; Kafka : [topic]")),
        ("lignes", z::int().min(0).max(100_000).optional().describe("N dernières lignes (défaut : 1000)")),
        ("depuis", z::int().min(1).optional().describe("Seulement les N dernières secondes")),
        ("debut", z::boolean().optional().describe("Kafka : tout le topic depuis le début")),
        ("precedent", z::boolean().optional().describe("Rancher : logs du conteneur précédent")),
        ("suivre", z::int().min(0).max(120).optional().describe("Suivre en direct N secondes de plus")),
        ("recherche", z::string().optional().describe("Texte (ou expression régulière) à chercher")),
        ("regex", z::boolean().optional()),
        ("casse", z::boolean().optional().describe("Respecter la casse")),
        ("niveaux", z::array(z::enumeration(&LEVELS)).optional().describe("Niveaux gardés (défaut : tous)")),
        ("max", max),
    ]
}

fn confirm() -> S { z::literal(json!(true)).describe("Obligatoire : confirme cette action irréversible") }

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() > max { format!("{}…", text.chars().take(max).collect::<String>()) } else { text.into() }
}

/// Une ligne pour l'agent : lisible, bornée.
fn line_of(entry: &LogEntry, raw: bool) -> Value {
    let mut v = obj! {
        "heure" => entry.parsed.time.map(crate::text::iso).or_else(|| entry.ts.clone()),
        "niveau" => entry.parsed.level.as_str(),
        "message" => clip(&entry.parsed.message, 2_000),
    };
    let m = v.as_object_mut().unwrap();
    if let Some(meta) = &entry.meta {
        m.insert("partition".into(), json!(meta.partition));
        m.insert("offset".into(), json!(meta.offset));
        if let Some(k) = &meta.key { m.insert("cle".into(), json!(k)); }
        if let Some(h) = &meta.headers { m.insert("headers".into(), json!(h)); }
    }
    if raw { m.insert("brut".into(), json!(clip(&entry.text, 10_000))); }
    v
}

/// Connexion telle qu'exposée : sans secret (config.yaml n'en contient pas).
fn describe_connection(c: &Connection, has_secret: bool) -> Value {
    let mut m = crate::connections::normalize(c);
    m.insert("secretEnregistre".into(), json!(has_secret));
    Value::Object(m)
}

fn path_of(a: &Value, key: &str) -> Vec<String> {
    a[key].as_array().map(|v| v.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn read_request(a: &Value, default_max: usize) -> ReadRequest {
    let debut = a.bool("debut").unwrap_or(false);
    ReadRequest {
        connection_id: a.string("connexion"),
        path: path_of(a, "chemin"),
        options: StreamOptions {
            follow: false,
            tail_lines: if debut { None } else { Some(a.int("lignes").unwrap_or(1_000) as u64) },
            since_seconds: a.int("depuis").map(|s| s as u64),
            previous: a.bool("precedent"),
        },
        follow_seconds: a.int("suivre").unwrap_or(0) as u64,
        search: a.str("recherche").filter(|r| !r.is_empty()).map(|text| SearchOptions {
            text,
            regex: a.bool("regex").unwrap_or(false),
            case_sensitive: a.bool("casse").unwrap_or(false),
        }),
        levels: a["niveaux"].as_array().map(|l| l.iter().filter_map(|v| v.as_str().and_then(Level::parse)).collect()),
        max_lines: a.int("max").map(|m| m as usize).unwrap_or(default_max),
    }
}

/// Opération S3 longue : attend sa fin (progression ignorée).
async fn s3_operation(app: &App, a: &Value, label: &str, op: S3Op) -> anyhow::Result<Content> {
    let (tx, rx) = tokio::sync::oneshot::channel::<S3Progress>();
    let tx = Arc::new(parking_lot::Mutex::new(Some(tx)));
    let sink = Arc::new(move |p: S3Progress| {
        if p.finished { if let Some(tx) = tx.lock().take() { let _ = tx.send(p); } }
    });
    if let Err(e) = app.s3.start(&a.string("connexion"), &a.string("bucket"), label, op, sink) { return Ok(fail(e.message)); }
    let p = rx.await?;
    Ok(match p.error {
        Some(e) => fail(e.message),
        None if p.cancelled => fail("Opération annulée."),
        None => ok(obj! { "termine" => true, "objets" => p.progress.done, "octets" => p.progress.bytes }),
    })
}

fn to_entry(v: &Value) -> S3Entry { S3Entry::from_key(v["cle"].as_str().unwrap_or_default()) }

fn s3_input(extra: Vec<(&'static str, S)>) -> Vec<(&'static str, S)> {
    let mut v = vec![("connexion", z::string()), ("bucket", z::string().min(1))];
    v.extend(extra);
    v
}

fn entries_s() -> S { z::array(z::object(vec![("cle", z::string().min(1).describe("Clé complète ; un dossier se termine par /"))])).min(1) }

pub fn core_tools() -> Vec<Tool> {
    let mut t = vec![];

    // --- Licence ------------------------------------------------------------------------------

    t.push(tool("licence_etat", "État de la licence", "État de l'activation d'Ocekour (clé, type de suivi, blocage éventuel).")
        .read_only().no_licence()
        .run(|app, _, _| async move { Ok(ok(app.licence_state())) }));

    t.push(tool("licence_activer", "Activer Ocekour", "Active Ocekour avec une clé d'activation (OCEKOUR1-…) ; ajoute sa préconfiguration éventuelle.")
        .input(vec![("cle", z::string().min(10).describe("Clé d'activation OCEKOUR1-…"))])
        .no_licence()
        .run(|app, a, _| async move { Ok(ok(app.activate(&a.string("cle")).await?)) }));

    // --- Connexions ---------------------------------------------------------------------------

    t.push(tool("connexions_lister", "Lister les connexions", "Connexions Rancher, Kafka et S3 configurées (sans aucun secret).")
        .read_only()
        .run(|app, _, _| async move {
            Ok(ok(app.config.config().connections.iter().map(|c| describe_connection(c, app.connections.has_secret(c.id()))).collect::<Vec<_>>()))
        }));

    t.push(tool("connexion_tester", "Tester une connexion", "Teste une connexion enregistrée (avec son secret enregistré).")
        .input(vec![("connexion", z::string())])
        .read_only()
        .run(|app, a, _| async move {
            let id = a.string("connexion");
            let result = async {
                let c = app.connections.get(&id)?;
                crate::connections::test_connection(&c, &app.connections.secret(&id)?).await
            }.await;
            Ok(match result { Ok(summary) => ok(summary), Err(e) => fail(e.message) })
        }));

    t.push(tool("connexion_enregistrer", "Créer ou modifier une connexion",
        "Crée (sans id) ou modifie (avec id) une connexion, au format de config.yaml (type rancher, kafka ou s3). Le secret (token, mot de passe, clé secrète) est chiffré à part.")
        .input(vec![
            ("connexion", z::record(z::any()).describe("Connexion au format de config.yaml (id facultatif pour une création)")),
            ("secret", z::string().optional().describe("Token, mot de passe ou clé secrète")),
        ])
        .run(|app, a, _| async move {
            let mut draft = a["connexion"].clone();
            if let Some(secret) = a.str("secret") { draft["secret"] = json!(secret); }
            Ok(ok(json!({ "id": app.connections.save(&draft)? })))
        }));

    t.push(tool("connexion_supprimer", "Supprimer une connexion", "Supprime une connexion et son secret.")
        .input(vec![("connexion", z::string()), ("confirmer", confirm())])
        .destructive()
        .run(|app, a, _| async move {
            let id = a.string("connexion");
            let name = app.connections.get(&id)?.name().to_string();
            app.connections.delete(&id)?;
            Ok(ok(format!("Connexion « {name} » supprimée.")))
        }));

    // --- Parcourir ----------------------------------------------------------------------------

    t.push(tool("sources_parcourir", "Parcourir une source",
        "Enfants d’un nœud : Rancher [cluster] → namespaces, [cluster, ns] → pods, [cluster, ns, pod] → conteneurs ; Kafka [] → topics ; S3 [] → buckets. Les clusters Rancher sont dans connexions_lister.")
        .input(vec![("connexion", z::string()), ("chemin", z::array(z::string()).max(3).default(json!([])))])
        .read_only()
        .run(|app, a, _| async move {
            Ok(match crate::sources::browse(&app.connections, &a.string("connexion"), &path_of(&a, "chemin")).await {
                Ok(nodes) => ok(nodes),
                Err(e) => fail(e.message),
            })
        }));

    // --- Logs et messages Kafka ---------------------------------------------------------------

    let mut lire = log_input(z::int().min(1).max(5_000).optional().describe("Lignes renvoyées au plus (défaut : 200)"));
    lire.push(("brut", z::boolean().optional().describe("Inclure la ligne brute")));
    t.push(tool("logs_lire", "Lire des logs ou des messages Kafka",
        "Lit les logs d'un conteneur Rancher ou les messages d'un topic Kafka (sans groupe ni commit), analysés comme dans l’interface (heure, niveau, message, profils), avec recherche et filtre de niveaux.")
        .input(lire)
        .read_only()
        .run(|app, a, _| async move {
            let r = read_logs(&app, read_request(&a, 200)).await?;
            let raw = a.bool("brut").unwrap_or(false);
            Ok(ok(obj! {
                "etat" => r.status.as_str(), "message" => r.message, "profil" => r.profile, "recues" => r.received,
                "renvoyees" => r.entries.len(),
                "lignes" => r.entries.iter().filter(|e| !e.notice).map(|e| line_of(e, raw)).collect::<Vec<_>>(),
            }))
        }));

    let mut exporter = log_input(z::int().min(1).max(1_000_000).optional().describe("Lignes au plus (défaut : toutes)"));
    exporter.push(("fichier", z::string().describe("Chemin absolu du fichier à écrire")));
    exporter.push(("ecraser", z::boolean().optional().describe("Remplacer le fichier s’il existe")));
    t.push(tool("logs_exporter", "Exporter des logs dans un fichier",
        "Comme logs_lire, mais écrit les lignes dans un fichier local : NDJSON (.ndjson, .jsonl, .json) ou texte brut (.log, .txt).")
        .input(exporter)
        .run(|app, a, _| async move {
            let file = a.string("fichier");
            if !Path::new(&file).is_absolute() { return Ok(fail("Chemin absolu attendu.")); }
            if Path::new(&file).exists() && !a.bool("ecraser").unwrap_or(false) {
                return Ok(fail("Le fichier existe déjà (ecraser: true pour le remplacer)."));
            }
            let r = read_logs(&app, read_request(&a, 1_000_000)).await?;
            let format = ExportFormat::for_file(&file);
            tokio::fs::write(&file, export_content(&r.entries, format)).await?;
            Ok(ok(json!({ "fichier": file, "format": format.as_str(), "lignes": r.entries.len() })))
        }));

    // --- Profils ------------------------------------------------------------------------------

    t.push(tool("profils_lister", "Lister les profils", "Profils d’affichage de config.yaml (champs, colonnes, champs en tête…).")
        .read_only()
        .run(|app, _, _| async move { Ok(ok(app.config.state().config["profiles"].clone())) }));

    t.push(tool("profil_modifier", "Modifier un profil",
        "Même action que les menus ⋯ du détail : profil `match` (cluster/namespace/workload ou kafka/<connexion>/<topic>), créé s’il n’existe pas.")
        .input(vec![("match", z::string().min(1)), ("operation", crate::settings::profile_edit_schema())])
        .run(|app, a, _| async move {
            let m = a.string("match");
            let state = crate::settings::edit_profile(&app.config, &m, &a["operation"])?;
            Ok(ok(state.config["profiles"].as_array().and_then(|p| p.iter().find(|p| p["match"] == json!(m))).cloned()))
        }));

    // --- S3 -----------------------------------------------------------------------------------

    t.push(tool("s3_lister", "Lister un dossier S3", "Dossiers et fichiers d’un dossier (préfixe « a/b/ », « » pour la racine).")
        .input(s3_input(vec![
            ("prefixe", z::string().default(json!(""))),
            ("recherche", z::string().optional().describe("Début de nom")),
            ("suite", z::string().optional().describe("Jeton de la page suivante")),
        ]))
        .read_only()
        .run(|app, a, _| async move {
            let r = app.s3.list(&a.string("connexion"), &a.string("bucket"), &a.string("prefixe"), &a.string("recherche"), a.str("suite").as_deref()).await;
            Ok(match r { Ok(page) => ok(page), Err(e) => fail(e.message) })
        }));

    t.push(tool("s3_apercu", "Aperçu d’un fichier S3", "Début d’un fichier (5 Mo au plus, .gz décompressé) : texte, ou type d’image.")
        .input(s3_input(vec![("cle", z::string().min(1))]))
        .read_only()
        .run(|app, a, _| async move {
            Ok(match app.s3.preview(&a.string("connexion"), &a.string("bucket"), &a.string("cle")).await {
                Ok(mut p) => {
                    let image = p.data_url.take().is_some();
                    let mut v = serde_json::to_value(p)?;
                    if image { v["image"] = json!("aperçu non transmis"); }
                    ok(v)
                }
                Err(e) => fail(e.message),
            })
        }));

    t.push(tool("s3_telecharger", "Télécharger depuis S3", "Télécharge des fichiers ou dossiers (récursif) dans un dossier local.")
        .input(s3_input(vec![("elements", entries_s()), ("dossier", z::string())]))
        .run(|app, a, _| async move {
            let dir = a.string("dossier");
            if !Path::new(&dir).is_absolute() { return Ok(fail("Chemin absolu attendu.")); }
            let entries = a["elements"].as_array().unwrap().iter().map(to_entry).collect();
            s3_operation(&app, &a, "téléchargement (MCP)", S3Op::Download { entries, file: None, dir: Some(PathBuf::from(dir)) }).await
        }));

    t.push(tool("s3_envoyer", "Envoyer vers S3", "Envoie des fichiers ou dossiers locaux (récursif) dans un dossier du bucket.")
        .input(s3_input(vec![("chemins", z::array(z::string()).min(1)), ("prefixe", z::string().default(json!("")))]))
        .run(|app, a, _| async move {
            let prefix = a.string("prefixe");
            if !prefix.is_empty() && !prefix.ends_with('/') { return Ok(fail("Préfixe de dossier attendu (…/).")); }
            let paths = path_of(&a, "chemins").into_iter().map(PathBuf::from).collect();
            s3_operation(&app, &a, "envoi (MCP)", S3Op::Upload { prefix, paths }).await
        }));

    t.push(tool("s3_renommer", "Renommer dans S3", "Renomme un fichier ou un dossier (copie puis suppression) dans son dossier.")
        .input(s3_input(vec![("cle", z::string().min(1)), ("nouveauNom", z::string().min(1))]))
        .destructive()
        .run(|app, a, _| async move {
            let entry = S3Entry::from_key(&a.string("cle"));
            s3_operation(&app, &a, "renommage (MCP)", S3Op::Rename { entry, new_name: a.string("nouveauNom") }).await
        }));

    t.push(tool("s3_supprimer", "Supprimer dans S3", "Supprime définitivement des fichiers et des dossiers (récursif).")
        .input(s3_input(vec![("elements", entries_s()), ("confirmer", confirm())]))
        .destructive()
        .run(|app, a, _| async move {
            let entries = a["elements"].as_array().unwrap().iter().map(to_entry).collect();
            s3_operation(&app, &a, "suppression (MCP)", S3Op::Remove { entries }).await
        }));

    // --- Liens de partage ---------------------------------------------------------------------

    t.push(tool("lien_ouvrir", "Ouvrir un lien de partage",
        "Décode un lien ocekour://open?d=… : connexion locale correspondante, chemin, options et filtres, à réutiliser avec logs_lire ou s3_lister.")
        .input(vec![("lien", z::string())])
        .read_only()
        .run(|app, a, _| async move {
            let link = share::decode(&a.string("lien"))?;
            let config = app.config.config();
            let Some(found) = share::find_connection(&link, &config.connections) else {
                let where_ = if link["t"] == "kafka" {
                    link["brokers"].as_array().map(|b| b.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default()
                } else { link["url"].as_str().unwrap_or_default().to_string() };
                return Ok(fail(format!("Aucune connexion locale pour ce lien ({where_}) : créez-la avec connexion_enregistrer.")));
            };
            let f = &link["filters"];
            let chemin = match link["t"].as_str() {
                Some("rancher") => json!([link["clusterId"], link["namespace"], link["pod"], link["container"]]),
                Some("kafka") => json!([link["topic"]]),
                _ => json!([link["bucket"]]),
            };
            Ok(ok(obj! {
                "connexion" => found.id(), "type" => found.kind(), "chemin" => chemin,
                "prefixe" => link["prefix"], "options" => link["options"],
                "recherche" => if f.is_object() { json!({ "text": f["text"], "caseSensitive": f["caseSensitive"], "regex": f["regex"] }) } else { Value::Null },
                "niveaux" => f["levels"],
            }))
        }));

    t.push(tool("lien_creer", "Créer un lien de partage",
        "Lien ocekour://… (sans secret) vers des logs, un topic ou un dossier S3, à coller dans Ocekour par un collègue.")
        .input(vec![
            ("connexion", z::string()),
            ("chemin", z::array(z::string().min(1)).min(1).max(4)),
            ("prefixe", z::string().optional().describe("S3 : dossier")),
            ("lignes", z::int().optional()),
            ("recherche", z::string().optional()),
        ])
        .read_only()
        .run(|app, a, _| async move {
            let c = app.connections.get(&a.string("connexion"))?;
            let chemin = path_of(&a, "chemin");
            let options = json!({ "follow": true, "tailLines": a.int("lignes").unwrap_or(1_000) });
            let filters = a.str("recherche").filter(|r| !r.is_empty()).map(|text| json!({ "text": text, "caseSensitive": false, "regex": false, "mode": "filter" }));
            let link = match &c {
                Connection::Rancher(r) => {
                    let get = |i: usize| chemin.get(i).cloned().unwrap_or_default();
                    let (cluster_id, namespace, pod, container) = (get(0), get(1), get(2), get(3));
                    let workload = workload_of(&app, &r.id, &cluster_id, &namespace, &pod).await.unwrap_or(pod.clone());
                    let cluster_name = r.clusters.iter().find(|k| k.id == cluster_id).map(|k| k.name.clone()).unwrap_or(cluster_id.clone());
                    share::link_for_container(&r.url, &cluster_id, &cluster_name, &namespace, &pod, &container, &workload, options, filters)
                }
                Connection::Kafka(k) => share::link_for_topic(&k.brokers, &chemin[0], options, filters),
                Connection::S3(s) => share::link_for_bucket(&s.url, &chemin[0], &a.string("prefixe")),
            };
            Ok(ok(share::encode(&link)))
        }));

    // --- Configuration ------------------------------------------------------------------------

    t.push(tool("configuration_lire", "Lire config.yaml", "Contenu de config.yaml (aucun secret n’y figure) et erreurs éventuelles.")
        .read_only()
        .run(|app, _, _| async move {
            let state = app.config.state();
            Ok(ok(json!({ "fichier": state.path, "contenu": app.config.read_text().unwrap_or_default(), "erreurs": state.errors })))
        }));

    // --- Attente -----------------------------------------------------------------------------

    t.push(tool("attendre", "Attendre un délai",
        "Attend un délai avant de rendre la main (utile pour espacer des relectures de logs, un sondage, une boucle). Au plus 300 s. Attention : le client MCP coupe la requête au-delà de son propre délai d’expiration (souvent ~60 s) : préférez des attentes courtes.")
        .input(vec![
            ("secondes", z::int().min(0).max(300).optional().describe("Secondes à attendre (0–300)")),
            ("minutes", z::number().min(0).max(5).optional().describe("Minutes à attendre (0–5)")),
        ])
        .read_only().no_licence()
        .run(|_, a, _| async move {
            let total = (a.int("secondes").unwrap_or(0) as f64 + a["minutes"].as_f64().unwrap_or(0.0) * 60.0).round() as i64;
            if total <= 0 { return Ok(fail("Indiquez un délai positif (secondes et/ou minutes).")); }
            if total > 300 { return Ok(fail("Délai trop long : 300 s au plus.")); }
            tokio::time::sleep(std::time::Duration::from_secs(total as u64)).await;
            Ok(ok(json!({ "attendu": total, "unite": "secondes" })))
        }));

    t
}

/// Tous les outils : cœur, services, contexte, web, bases de données.
pub fn all() -> Vec<Tool> {
    let mut t = core_tools();
    t.extend(crate::services::tools::all());
    t.extend(crate::web::tools::tools());
    t.extend(crate::bdd::tools::tools());
    t.extend(crate::contexte::tools::tools());
    t
}
