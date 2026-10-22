# Historique des versions

[← Sommaire](README.md)

## 2.2.0

**Nouveau : sonde, les logs d'une machine distante** ([guide](sonde.md))

- `ocekour-headless sonde` expose en lecture seule les fichiers de logs d'un serveur Windows ou
  Linux (binaire Linux statique, sans dépendance) : HTTPS, jeton par sonde, certificat épinglé
  côté poste, plusieurs postes en même temps, reprise après coupure.
- Nouvelle connexion **Sonde** : sources dans l'arbre, onglets, assemblage, liens de partage,
  outils MCP.
- Service Windows ou unité systemd : `sonde service installer`.

## 2.1.0

**Nouveau : fichiers locaux** ([guide](fichiers-locaux.md))

- Ouvrir un fichier de logs du poste : Ctrl+O, glisser-déposer, ligne de commande
  (`Ocekour.exe C:\logs\app.log`).
- **Motifs** (`app-*.log`) : suivi du fichier le plus récent, avec bascule à la rotation.
- Section **Fichiers locaux** (récents, favoris), choix de l'encodage (Windows-1252…).
- Agents : accès limité aux fichiers déclarés dans **Paramètres → Fichiers pour les agents**
  (outil `fichiers_lister`).

## 2.0.0

**Nouvelle base : Rust + Tauri, interface Svelte**

- Ocekour est réécrit : un **cœur en Rust** (sources, flux, Kafka, S3, MCP, notifications,
  contexte IA, bases de données, navigateur) et une **interface Svelte** qui n'en est qu'une
  surcouche, affichée par **WebView2** (déjà présent sur Windows 10/11).
- **Démarrage immédiat** : plus de décompression de ~330 Mo au premier lancement ; exe d'environ
  30 Mo.
- **Sans interface** : `Ocekour-<version>.exe --mcp | --mcp-http | --notify` comme avant, et un
  `ocekour-headless.exe` autonome (sans WebView2) pour les serveurs et tâches planifiées.
- Mêmes fichiers (`%APPDATA%\Ocekour\config.yaml`, `secrets.json`, `secret.key`, journaux),
  mêmes clés d'activation, mêmes outils MCP : rien à refaire en passant de la 1.x à la 2.0.

## 1.3.1

**Notices regroupées**

- Quand la ligne « Flux interrompu » (ou une autre notice d'Ocekour) se répète, elle n'est plus
  affichée en double : une seule ligne apparaît avec le **nombre d'occurrences** (ex. `×12`).
  Cliquez sur ce nombre pour **afficher toutes les occurrences** (avec leur heure), et à nouveau
  pour replier.

## 1.3.0

**Nouveau : bases de données en lecture seule (outils MCP `bdd_*`)**

- Le serveur MCP peut donner aux agents un accès **en lecture seule** à vos bases **PostgreSQL,
  SQL Server et Oracle** : explorer schémas, tables et colonnes, et lancer des `SELECT`. Toute
  écriture (INSERT/UPDATE/DELETE/DDL…) est **refusée** (garde SQL + transaction read-only + bornes).
  Outils `bdd_connexions`, `bdd_tables`, `bdd_colonnes`, `bdd_apercu`, `bdd_requete`,
  `bdd_expliquer`. Voir [bdd.md](bdd.md).
- **Tous les drivers inclus**, 100 % JavaScript (rien à installer) : `pg` (PostgreSQL 8→17+),
  `tedious` (SQL Server 2005→2022, Azure), `oracledb` en mode **thin** (Oracle 12.1→23ai, sans
  Instant Client). Connexions gérées dans **Paramètres → Bases de données** ; mot de passe chiffré
  dans `secrets.json`, jamais dans `config.yaml`.

## 1.2.0

**Nouveau : accès web & contrôle de navigateur (outils MCP `web_*`)**

- Le serveur MCP peut exposer à vos agents un **accès au web** et le **pilotage d'un navigateur**
  (modèle Microsoft Playwright MCP, par **snapshot d'accessibilité** : chaque élément a une ref).
  Outils : `web_recuperer` (URL → Markdown, sans navigateur), `web_naviguer`, `web_apercu`,
  `web_texte`, `web_cliquer`, `web_saisir`, `web_choisir`, `web_touche`, `web_patienter`,
  `web_capture`, `web_onglets`, `web_fermer`. Voir [web.md](web.md).
- Pilote le navigateur **déjà installé** (Edge par défaut, ou Chrome) via Playwright : **aucun
  binaire téléchargé** (postes cloisonnés). **Désactivé par défaut** (`web.enabled` dans
  `config.yaml`) ; fenêtre visible par défaut.

## 1.1.0

**Filtres enregistrés & encart rétractable**

- L'encart **Filtres** se **replie / déplie** (chevron) et borne sa hauteur : les logs restent
  visibles même avec beaucoup de règles.
- On peut **nommer**, **enregistrer** et **réutiliser** un jeu de règles entre **tous les onglets**
  (pods, flux, assemblages). Les filtres enregistrés vivent dans `config.yaml` (`filterPresets`),
  conservés entre deux lancements. Voir [filtres.md](filtres.md).

## 1.0.0

**Nouveau : encart Filtres (règles par champ)**

- Chaque onglet de logs a un encart **Filtres** : des règles cumulables qui **masquent** (liste
  noire) ou **gardent seules** (liste blanche) des lignes selon la valeur d'un **champ** — `path`,
  `http.path`, `logger`…, ou `@message` / `@line` — en **exact**, **contient** ou **regex**. Voir
  [filtres.md](filtres.md).
- Règles mémorisées par onglet (rouvertes au démarrage), appliquées aussi aux **assemblages** et au
  **téléchargement filtré**. Raccourci : pastille du nombre de règles actives, bascule par règle.

## 0.9.1

**Nouvel outil MCP : `attendre`**

- Les agents peuvent attendre un délai (secondes/minutes, **300 s au plus**) pour espacer des
  relectures de logs ou piloter une boucle. L'attente tient la requête ouverte sans bloquer les
  autres appels ; au-delà du délai d'expiration propre au client MCP (souvent ~60 s), c'est le
  client qui coupe — préférez des attentes courtes.

## 0.9.0

**Nouveau : Télécharger l'historique complet**

- Bouton **Télécharger…** dans un onglet de logs : récupère l'**historique complet depuis la
  création** du/des pod(s) (pas seulement ce qui est chargé en mémoire), au choix **tout** ou
  **filtré** (recherche, niveaux, et sources masquées pour un assemblage). Voir
  [partage-export.md](partage-export.md).
- Conteneur redémarré : les logs de l'**instance précédente** sont inclus quand disponibles.
- **Assemblage** : toutes les sources sont récupérées et **fusionnées par horodatage**, chaque
  ligne portant sa source (préfixe `[source]` en texte, champ `source` en NDJSON).

## 0.8.1

**Correction (Assemblage)**

- Quand on **ajoute une source** à un assemblage, son historique est désormais **fusionné par
  horodatage** avec les lignes déjà affichées, au lieu d'être ajouté à la fin (le tampon est
  ré-ordonné dès qu'une ligne plus ancienne arrive, tant que la fenêtre de réordonnancement n'est
  pas à 0).

## 0.8.0

**Nouveau : Assemblage (vue fusionnée multi-sources)**

- Un onglet d'**assemblage** fusionne **plusieurs flux** (conteneurs Rancher + topics Kafka,
  clusters et connexions mélangés) dans **une seule liste triée dans le temps**, chaque ligne
  marquée de sa **source** (badge coloré). Voir [assemblage.md](assemblage.md).
- **Filtre par source**, **recherche / niveaux / pause / export** sur la vue fusionnée.
- **Ordre** : tri temporel en historique ; en direct, **fenêtre de réordonnancement** réglable
  (2 s par défaut, 0 = ordre d'arrivée).
- **Création** : bouton ⊕ Assemblage, puis clic sur les conteneurs / topics pour les ajouter
  (jusqu'à 10) ; retrait et renommage ; rouvert au démarrage.

## 0.7.1

**Corrections du Contexte IA**

- **Détection de Qwen Code** : Ocekour lit la vraie structure de `~/.qwen/settings.json`
  (`modelProviders` + `security.auth.selectedType` + `model.name`, clé résolue via `envKey` dans
  le bloc `env`) — le cas d'un endpoint OpenAI-compatible **vLLM auto-hébergé**. Les formes
  simples et `~/.qwen/.env` restent acceptées. La configuration était sinon vue comme introuvable.
- La **détection du LLM s'affiche immédiatement** dans Paramètres → Contexte IA au changement de
  réglage (avant, seulement pendant un passage d'ingestion).
- **TLS de l'endpoint LLM** : on peut choisir _Ignorer le certificat_ (https auto-signé) ou
  indiquer un **certificat d'autorité**, comme pour Rancher et les Comptes.

## 0.7.0

**Nouveau : Contexte IA, une mémoire locale pour vos agents**

- Ocekour peut tenir une **mémoire de contexte** (répertoire de fichiers Markdown, **sans base
  de données ni vecteurs**) alimentée en continu : chaque **message Rocket.Chat de vos salons**
  (reçu **comme envoyé**) et chaque nouveauté **GitLab / Jira / Confluence** vous concernant.
  Voir [contexte-ia.md](contexte-ia.md).
- **Ingestion** par un graphe **LangGraph** embarqué (motif « ambient agent »), à intervalle
  régulier. Désactivée par défaut ; à la première activation, part de « maintenant » (pas de
  rattrapage).
- **Exposée aux agents par le MCP** : outils `contexte_chercher`, `contexte_recents`,
  `contexte_lire` (lecture seule). L'agent ne lit pas les fichiers directement.
- **Résumé** facultatif par le **LLM déjà configuré** dans Qwen Code ou Mistral Vibe (lu dans
  leur config ; aucune clé stockée). Sinon, texte brut.
- **Bouton « Configurer mon agent »** : enregistre le serveur MCP d'Ocekour dans
  `~/.qwen/settings.json` ou `~/.vibe/config.toml` (sauvegarde `.bak`, idempotent).

## 0.6.0

**Notifications Windows et commentaires GitLab**

- Les nouveautés s'affichent aussi en **notifications système Windows**, en plus du message
  Rocket.Chat (réglage _Afficher aussi les nouveautés en notifications Windows_). Fonctionne
  aussi en mode `--notify`.
- Nouvelles notifications GitLab : **commentaire sur une MR que vous avez écrite** et
  **réponse à un de vos commentaires**.

## 0.5.0

**Changement : les secrets ne dépendent plus de Windows**

- Les tokens, mots de passe et la clé d'activation sont désormais chiffrés par une **clé
  locale** (`secret.key`, AES-256-GCM) au lieu du chiffrement Windows (DPAPI). Un changement
  de session Windows (mot de passe de domaine, profil) ne les rend donc **plus illisibles**.
- **Migration automatique** : au premier lancement de la 0.5.0, les secrets encore lisibles
  écrits par une version antérieure sont convertis vers la clé locale. Ceux qui étaient déjà
  cassés par DPAPI sont à ressaisir une fois.
- Compromis assumé : `secret.key` étant sur le même poste que `secrets.json`, la protection
  n'est plus liée à la session Windows. Les secrets restent chiffrés au repos (jamais en
  clair), mais ne copiez pas ces fichiers sur un poste partagé sans précaution.

## 0.4.1

**Corrigé**

- Après un **changement de session Windows** pendant un week-end / une mise à jour de mot de
  passe de domaine, les secrets enregistrés (jetons, clé, mot de passe de connexion) peuvent
  devenir indéchiffrables (DPAPI). Ocekour ne **plante plus** dans ce cas : un secret illisible
  est traité comme absent (à ré-enregistrer), le **jeton du serveur MCP est régénéré
  automatiquement**, et le journal indique clairement quoi ressaisir (clé d'activation,
  jetons dans Paramètres → Comptes, identifiants de connexion).

## 0.4.0

**Nouveau : notifications vers Rocket.Chat**

- Ocekour surveille **GitLab**, **Jira** et **Confluence** et vous envoie, dans Rocket.Chat
  (à vous-même par défaut), **un message groupé** dès qu'il y a du nouveau qui vous concerne
  directement : assigné, mention, revue demandée, pipeline en échec, commentaire, changement
  de statut… Voir [Notifications](notifications.md).
- Réglages dans **Paramètres → Notifications** : activation, intervalle, destinataire,
  sources, message d'essai.
- Mode **`--notify`** : Ocekour tourne sans interface, avec seulement les notifications.
- Réutilise les comptes du hub ; aucun secret ne circule ; anti-doublon même après
  redémarrage ; n'emprunte pas le chemin d'écriture des agents.

## 0.3.0

**Nouveau : Ocekour, hub MCP des agents**

- Le serveur MCP donne accès à **GitLab**, **Jira**, **Confluence** et **Rocket.Chat**
  (instances de l'entreprise), avec **votre jeton**, donc vos droits. Voir
  [Hub MCP](hub-mcp.md).
- **Paramètres → Comptes** : URL de chaque outil, bouton **Créer un jeton ↗** qui ouvre la
  bonne page (pré-remplie pour GitLab), explications pas à pas, test du jeton. Les jetons
  sont chiffrés sur le poste, jamais dans `config.yaml`.
- Lecture : projets, fichiers, MR, diffs, pipelines et logs de jobs ; tickets (JQL),
  sprints ; pages Confluence en Markdown ; salons et fils Rocket.Chat.
- Écriture, **à autoriser outil par outil** (désactivée par défaut) : commentaires de MR
  **globaux ou positionnés sur une ligne**, issues, MR, relances de pipeline ; tickets,
  commentaires, statuts, temps passé ; pages et commentaires Confluence ; messages
  Rocket.Chat.
- Toute écriture est **signée « via Ocekour »** et inscrite dans la nouvelle page **Actions
  de l'agent** (lien vers l'élément, client MCP, paramètres).
- La clé d'activation peut **pré-remplir les URL** de ces outils.

**Serveur MCP**

- Aide à la connexion pour **Mistral Vibe** (Paramètres et [documentation](mcp.md)).

**Performances** (mesures sur un poste lent simulé, avec 100 000 lignes)

- **Démarrage** : l'exe portable ne décompresse plus l'application à chaque lancement,
  seulement au premier lancement d'une version (`%LOCALAPPDATA%\Ocekour\app\`). Il n'écrit
  plus ~750 Mo sur le disque à chaque ouverture. L'interface charge 3 fois moins de code au
  démarrage, et Kubernetes, Kafka et le serveur MCP ne sont chargés qu'à leur premier usage.
- **Logs en direct** (~1 000 lignes/s) : environ 50 images/s au lieu de 7, l'interface
  reste fluide. Avant, chaque lot reçu refiltrait tout l'onglet.
- **Défilement** 5 fois plus fluide, **chargement** de 100 000 lignes en 3,7 s au lieu de
  6 s, **recherche** 2 fois plus rapide (appliquée après une courte pause de frappe).
- Caches de Chromium déplacés dans `%LOCALAPPDATA%` : ils ne font plus partie du profil
  itinérant. Correcteur orthographique désactivé : il n'y a plus de téléchargement de
  dictionnaires au démarrage.
- Dans la liste, une cellule affiche au plus 400 caractères et les retours à la ligne
  apparaissent comme « ↵ ». Le texte complet reste dans le détail.

## 0.2.0

**Nouveau : clé d'activation**

- Ocekour demande une **clé d'activation** au premier lancement : elle vous est remise par
  l'administrateur. La clé est gardée chiffrée sur le poste, et **Paramètres → Licence**
  permet d'en changer.
- La clé fixe le suivi : **sans suivi**, **suivi si possible** ou **suivi obligatoire**. Avec
  le suivi obligatoire, Ocekour doit joindre le serveur de statistiques de l'équipe (écran
  « Réessayer » sinon).
- La configuration des statistiques vient de la clé : la case « Envoyer des statistiques »
  des Paramètres disparaît.
- La clé peut **préconfigurer les sources** de l'équipe (connexions Rancher, Kafka, S3 et
  profils). Elles sont ajoutées à l'activation, sans secret : il reste à saisir vos
  identifiants.

**Confort**

- La liste des connexions (colonne de gauche) se redimensionne en glissant son bord. La
  largeur est mémorisée.
- Colonnes JSON : le bouton **{}** d'un en-tête de colonne affiche le JSON indenté, sur
  plusieurs lignes. Le réglage est enregistré dans le profil du workload ou du topic.

**Serveur MCP**

- Ocekour intègre un **serveur MCP** : un agent (Claude Code, Claude Desktop…) fait les
  mêmes actions que l'interface (connexions, logs, Kafka, S3, profils, liens). Il s'active
  dans Paramètres → Serveur MCP, en HTTP local avec jeton.
- **Mode sans interface** : `--mcp-http` (serveur en arrière-plan) ou `--mcp` (stdio). Voir
  [Serveur MCP](mcp.md).
- Aide à la connexion **par client** (Claude Code, Qwen Code, autres), dans les Paramètres
  et dans la documentation : Qwen Code attend `httpUrl` et l'option `-H`.
- Et une petite surprise pour les amateurs de codes de triche…

**Corrigé**

- La fermeture d'Ocekour pouvait rester bloquée (introduit en 0.1.1 avec les statistiques).

## 0.1.1

**Corrigé**

- Le **journal** `%APPDATA%\Ocekour\logs\main.log` n'était pas écrit : il est désormais
  créé au démarrage. Il contient le démarrage (version, système), les connexions créées,
  modifiées ou supprimées, l'ouverture et la fermeture des flux, les reconnexions, les
  erreurs (tests de connexion, flux, S3, messages d'erreur affichés) et les opérations S3.
  **Aucun secret** n'y est écrit. Taille limitée à 5 Mo, puis `main.old.log`.
- Paramètres : description du « Dossier des mises à jour » corrigée (fonction pas encore
  disponible).

**Ajouté**

- Paramètres → **Afficher le journal** : ouvre l'Explorateur sur `main.log`.
- **Statistiques d'utilisation anonymes** envoyées au Matomo de l'équipe : écrans, actions,
  codes d'erreur, version. Aucune donnée de vos sources ni aucun secret (voir
  [Configuration](configuration.md#statistiques-dutilisation)).

## 0.1.0 — première diffusion

**Sources**

- **Rancher** 2.5+ : token d'API ou import de kubeconfig, choix des clusters, arbre
  namespace › pod › conteneur avec état de santé, logs en direct ou en historique, logs du
  conteneur précédent, reconnexion sans doublon ni trou, pods de remplacement proposés
  après un redéploiement.
- **Kafka** : SASL_PLAINTEXT / SASL_SSL / PLAINTEXT, SCRAM-SHA-512/256, PLAIN ; lecture
  sans groupe ni commit ; N derniers messages, depuis une date, depuis le début, suivi en
  direct ; colonnes partition / offset / clé ; clé et headers dans le détail ; Avro via
  Schema Registry, JSON, texte, binaire.
- **S3** d'entreprise : explorateur (fil d'Ariane, tri, recherche par préfixe, pagination),
  aperçu (texte coloré, JSON indenté, `.gz`, images), téléchargement, envoi (dont
  glisser-déposer), renommage, suppression, avec progression et annulation.

**Lecture des logs**

- Détection automatique JSON / logfmt / texte, heure locale, niveaux.
- Recherche texte ou regex, modes filtrer / surligner, F3, filtres par niveau,
  « afficher le contexte ».
- Détail lisible : arbre JSON, champs multilignes, JSON imbriqué ; réglages en un clic
  enregistrés en profils (colonnes, champs en tête, champs heure / niveau / message).

**Confort**

- Onglets (glisser-déposer, raccourcis, compteur de nouvelles erreurs), restaurés au
  démarrage ; favoris ; thème clair / sombre.
- Export NDJSON ou texte des lignes affichées.
- Lien de partage `ocekour://…` sans secret, à coller dans Ocekour.
- Configuration `config.yaml` éditable dans l'application ; secrets chiffrés par Windows.

**Limites connues**

- Copie de plusieurs lignes à la fois : pas encore (une ligne à la fois depuis le détail).
- Détection automatique des mises à jour sur un partage réseau : prévue.
- Exe non signé : avertissement SmartScreen au premier lancement.
