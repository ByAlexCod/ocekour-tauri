# Spec — Ocekour, hub MCP des agents locaux (0.3.0)

> Statut : **validée, implémentée en 0.3.0** · Guide utilisateur : [hub-mcp.md](../guide/hub-mcp.md)
> · Suite de la spec MCP livrée en 0.2.0 ([guide](../guide/mcp.md))

## 1. Objectif

Ocekour devient **le point d'entrée MCP unique** des agents locaux des développeurs (Claude
Code, Qwen Code, Mistral Vibe…). En plus des logs Rancher, de Kafka et de S3, l'agent accède
avec **les droits de l'utilisateur** à :

| Service                    | Version cible         | Authentification                                          |
| -------------------------- | --------------------- | --------------------------------------------------------- |
| **GitLab** (auto-hébergé)  | inconnue, API v4      | jeton d'accès personnel (PAT), portée `api`               |
| **Jira** Data Center       | 9.12.1                | PAT (`Authorization: Bearer`)                             |
| **Confluence** Data Center | 9.2.6                 | PAT (`Authorization: Bearer`)                             |
| **Rocket.Chat**            | inconnue, API REST v1 | jeton personnel : `X-Auth-Token` + `X-User-Id` (sans 2FA) |

Principes (inchangés depuis la 0.1) :

- **Jetons générés par chaque utilisateur**, stockés chiffrés (clé locale, `secrets.json`),
  **jamais** dans `config.yaml`, jamais renvoyés par le MCP, jamais dans une clé ni dans un
  lien de partage.
- `config.yaml` ne contient que **les URL** et les réglages (TLS, interrupteurs). La clé
  d'activation peut les **pré-remplir**, sans les verrouiller.
- L'application **guide la création des jetons** : ouverture de la bonne page du serveur et
  explications claires.
- **Une seule instance par service**, **un seul endpoint** MCP (`/mcp`, et `--mcp` en stdio).
- **Toute écriture est signée** (visible comme venant d'Ocekour) et **journalisée localement**.
- Pas de confirmation humaine par action : le client MCP (Claude Code, etc.) gère déjà cette
  demande. Pour éviter les écritures, on désactive le service ou l'écriture (§5).

Hors périmètre 0.3.x : l'intégration de ces services dans l'interface (onglet GitLab, lien
entre un ticket Jira et les logs…), les outils combinés entre sources, l'agrégation d'autres
serveurs MCP, la troncature des réponses (l'agent pagine lui-même).

## 2. Découpage

Tout est livré en **0.3.0** (décision : pas de découpage en 0.3.1) : écran **Comptes**,
**GitLab**, **Jira**, **Confluence**, **Rocket.Chat**, **journal des actions**, interrupteurs,
Matomo, aide Mistral Vibe.

## 3. Configuration

### 3.1 `config.yaml`

Nouvelle section `services` (une entrée par service, toutes facultatives) :

```yaml
services:
  gitlab:
    url: https://gitlab.intra
    tls: { insecure: false, caFile: C:\certs\ac-interne.cer } # même réglage que Rancher
    enabled: true # outils visibles par l'agent
    write: false # outils d'écriture visibles par l'agent
  jira:
    url: https://jira.intra
  confluence:
    url: https://confluence.intra
  rocketchat:
    url: https://chat.intra
```

- `enabled` vaut `true` par défaut. Les outils d'un service n'apparaissent que si le service
  est activé **et** qu'un jeton est enregistré.
- `write` vaut `false` par défaut : **l'écriture est désactivée tant que l'utilisateur ne l'a
  pas activée** dans l'écran Comptes (un clic). Les outils d'écriture n'apparaissent pas du
  tout à l'agent, ce qui vaut mieux qu'un refus au moment de l'appel.

### 3.2 Secrets (`secrets.json`, clé locale)

| Id                   | Contenu                           |
| -------------------- | --------------------------------- |
| `service:gitlab`     | le PAT                            |
| `service:jira`       | le PAT                            |
| `service:confluence` | le PAT                            |
| `service:rocketchat` | `{ "userId": "…", "token": "…" }` |

Un changement d'URL **conserve** le jeton, qui peut toutefois échouer au test. Un bouton
« Oublier le jeton » l'efface.

### 3.3 Clé d'activation (préconfiguration)

La préconfiguration accepte une clé `services`, avec **seulement** `url` et `tls` par service.
Le contrôle anti-secret existant s'applique aussi à cette section. À l'activation, chaque
service **absent** de la configuration est ajouté. Un service déjà configuré n'est pas
modifié. Tout reste modifiable par l'utilisateur.

```yaml
# docs/exemples/preconfiguration.yaml (extrait)
services:
  gitlab: { url: https://gitlab.intra }
  jira: { url: https://jira.intra }
  confluence: { url: https://confluence.intra }
  rocketchat: { url: https://chat.intra }
```

## 4. Écran « Comptes » (Paramètres → Comptes)

Une carte par service :

```
┌ GitLab ─────────────────────────────────────────────────────────────┐
│ URL  [https://gitlab.intra              ]  TLS [ ] ignorer  [AC…]    │
│ Jeton ●●●●●●●●  ✓ connecté : Alex Martin (@amartin)                   │
│   [Créer un jeton ↗]  [Coller / remplacer]  [Tester]  [Oublier]        │
│ [x] Visible par l'agent     [ ] Autoriser l'écriture                   │
│ ▸ Comment créer le jeton ?                                             │
└─────────────────────────────────────────────────────────────────────┘
```

### 4.1 Création guidée du jeton

« Créer un jeton ↗ » ouvre la page dans le navigateur par défaut, pré-remplie quand le
serveur le permet. Le texte d'aide, dépliable, donne les étapes exactes.

| Service     | Page ouverte                                                                                                                             | Étapes (texte d'aide)                                                                                                                                            |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GitLab      | `/-/user_settings/personal_access_tokens?name=Ocekour&scopes=api` (lien de secours `/-/profile/personal_access_tokens` pour GitLab < 16) | Nom et portée `api` déjà remplis → choisir une date d'expiration → « Create » → copier le jeton affiché (une seule fois) → « Coller » dans Ocekour               |
| Jira        | `/secure/ViewProfile.jspa?selectedTab=com.atlassian.pats.pats-plugin:jira-user-personal-access-tokens`                                   | « Créer un jeton » → nom `Ocekour` → expiration au choix → copier                                                                                                |
| Confluence  | `/plugins/personalaccesstokens/usertokens.action`                                                                                        | idem Jira                                                                                                                                                        |
| Rocket.Chat | `/account/tokens`                                                                                                                        | Nom `Ocekour` → ne pas cocher « ignorer la double authentification » → « Ajouter » → copier **le jeton et l'identifiant utilisateur** (deux champs dans Ocekour) |

Si la page renvoie une erreur (droit « créer des jetons personnels » absent sur Rocket.Chat,
PAT désactivés par l'admin), le texte d'aide indique qui contacter.

### 4.2 Test du jeton

Le test est lancé à l'enregistrement et par « Tester ». Il affiche l'identité, ou une erreur
claire (401 : jeton invalide ou expiré ; certificat ; réseau).

| Service     | Appel                                                                                                                                   |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| GitLab      | `GET /api/v4/user`, puis `GET /api/v4/personal_access_tokens/self` (portées : avertit si `api` manque alors que l'écriture est activée) |
| Jira        | `GET /rest/api/2/myself`                                                                                                                |
| Confluence  | `GET /rest/api/user/current`                                                                                                            |
| Rocket.Chat | `GET /api/v1/me`                                                                                                                        |

Pas d'alerte avant expiration. Un appel refusé (401) pendant l'usage renvoie à l'agent
« jeton GitLab expiré ou révoqué : Ocekour → Paramètres → Comptes ».

## 5. Outils MCP

Même conventions qu'en 0.2.0 : noms en français préfixés par le service, annotations
`readOnlyHint` pour la lecture et `destructiveHint` pour l'écriture, résultats JSON. La
pagination est exposée telle quelle (`page`, `parPage`), sans troncature par Ocekour.
L'agent est responsable du volume qu'il demande.

Les **projets et tickets sont désignés comme dans l'outil web** : chemin `groupe/projet` ou
id pour GitLab, clé `PAY-123` pour Jira, et une URL web collée est acceptée partout. Chaque
résultat contient l'**URL web** de l'élément.

### 5.1 GitLab (0.3.0)

**Lecture**

| Outil                                              | Rôle                                                                        |
| -------------------------------------------------- | --------------------------------------------------------------------------- |
| `gitlab_projets_chercher`                          | Recherche de projets (nom, groupe)                                          |
| `gitlab_projet`                                    | Détail : branche par défaut, URL, description                               |
| `gitlab_arborescence`                              | Fichiers d'un dossier (réf. = branche, tag ou commit)                       |
| `gitlab_fichier_lire`                              | Contenu d'un fichier à une réf.                                             |
| `gitlab_code_chercher`                             | Recherche de code (projet ou groupe)                                        |
| `gitlab_commits` / `gitlab_commit_lire`            | Historique (branche, chemin, auteur, dates) ; détail et diff d'un commit    |
| `gitlab_branches`                                  | Branches et tags                                                            |
| `gitlab_mr_lister`                                 | MR (état, auteur, relecteur, libellé)                                       |
| `gitlab_mr_lire`                                   | Description, statut, approbations, pipeline, `diff_refs`                    |
| `gitlab_mr_diff`                                   | Fichiers modifiés avec diff (paginé)                                        |
| `gitlab_mr_discussions`                            | Fils de discussion, avec la position (fichier, ligne) de chaque commentaire |
| `gitlab_issues_lister` / `gitlab_issue_lire`       | Issues et commentaires                                                      |
| `gitlab_pipelines_lister` / `gitlab_pipeline_lire` | Pipelines et jobs (statut, durée, étape)                                    |
| `gitlab_job_log`                                   | Log d'un job (paramètre `fin` : N dernières lignes)                         |

**Écriture** (visible seulement si l'écriture est activée)

| Outil                                              | Rôle                                                                      |
| -------------------------------------------------- | ------------------------------------------------------------------------- |
| `gitlab_mr_commenter`                              | Commentaire **global**, ou **positionné** sur une ligne (voir ci-dessous) |
| `gitlab_discussion_repondre`                       | Répondre dans un fil existant                                             |
| `gitlab_discussion_resoudre`                       | Résoudre ou rouvrir un fil                                                |
| `gitlab_mr_creer`                                  | Créer une MR (source, cible, titre, description, brouillon)               |
| `gitlab_issue_creer` / `gitlab_issue_commenter`    | Créer une issue, la commenter                                             |
| `gitlab_pipeline_relancer` / `gitlab_job_relancer` | Relancer un pipeline, un job                                              |

**Commentaire positionné**. Paramètre facultatif `position` :

```jsonc
{
  "fichier": "src/paiement/service.ts",
  "ligne": 42, // ligne dans la version indiquée par « cote »
  "cote": "nouveau", // nouveau (ligne ajoutée ou inchangée) | ancien (ligne supprimée)
  "ligneFin": 48, // facultatif : commentaire sur une plage de lignes
}
```

Ocekour récupère lui-même les `diff_refs` de la **dernière version** de la MR, calcule
`old_line` et `new_line` pour une ligne inchangée, puis crée la discussion
(`POST /projects/:id/merge_requests/:iid/discussions`, `position_type: text`, `line_range`
pour une plage). Si la ligne n'est pas dans le diff, l'erreur le dit clairement
(« ligne 42 hors des modifications de service.ts : commentez une ligne du diff ou faites un
commentaire global ») au lieu de laisser GitLab la refuser en silence. Sans `position`, le
commentaire est global.

Approuver et merger une MR sont **exclus** (pas demandés ; ajoutables plus tard).

### 5.2 Jira 9.12 (0.3.0)

API REST v2 et Agile 1.0.

| Outil                                                    | Rôle                                                                             | Type     |
| -------------------------------------------------------- | -------------------------------------------------------------------------------- | -------- |
| `jira_chercher`                                          | Recherche **JQL** (champs choisis, pagination)                                   | lecture  |
| `jira_ticket_lire`                                       | Ticket complet : champs, commentaires, liens, sous-tâches, pièces jointes (noms) | lecture  |
| `jira_projets`                                           | Projets, types de ticket, champs obligatoires à la création                      | lecture  |
| `jira_transitions`                                       | Transitions possibles d'un ticket                                                | lecture  |
| `jira_utilisateurs`                                      | Recherche d'utilisateurs (identifiant pour l'assignation)                        | lecture  |
| `jira_tableaux` / `jira_sprints` / `jira_sprint_tickets` | Tableaux, sprints et contenu d'un sprint                                         | lecture  |
| `jira_ticket_creer`                                      | Créer (projet, type, résumé, description, champs)                                | écriture |
| `jira_ticket_modifier`                                   | Modifier des champs                                                              | écriture |
| `jira_commenter`                                         | Ajouter un commentaire                                                           | écriture |
| `jira_transition`                                        | Changer de statut (avec commentaire facultatif)                                  | écriture |
| `jira_assigner`                                          | Assigner (utilisateur ou soi-même)                                               | écriture |
| `jira_temps_saisir`                                      | Saisir du temps passé (worklog)                                                  | écriture |

Format du texte : l'agent écrit en **Markdown**, qu'Ocekour convertit en wiki-markup Jira.
Le paramètre `format: "wiki"` permet d'envoyer le texte tel quel. À la lecture, le texte Jira
est renvoyé tel quel (wiki-markup).

### 5.3 Confluence 9.2.6

| Outil                       | Rôle                                                                                                  | Type     |
| --------------------------- | ----------------------------------------------------------------------------------------------------- | -------- |
| `confluence_chercher`       | Recherche **CQL** (ou texte simple converti en CQL)                                                   | lecture  |
| `confluence_page_lire`      | Page convertie en **Markdown** (ou `format: "stockage"`), avec version, auteur, espace, parents       | lecture  |
| `confluence_espace`         | Arborescence des pages d'un espace                                                                    | lecture  |
| `confluence_pieces_jointes` | Liste des pièces jointes (et contenu texte si lisible)                                                | lecture  |
| `confluence_page_creer`     | Créer une page (espace, parent, titre, contenu Markdown)                                              | écriture |
| `confluence_page_modifier`  | Modifier une page (version incrémentée automatiquement ; refus si la page a changé depuis la lecture) | écriture |
| `confluence_commenter`      | Commenter une page                                                                                    | écriture |

### 5.4 Rocket.Chat

| Outil                   | Rôle                                                                               | Type     |
| ----------------------- | ---------------------------------------------------------------------------------- | -------- |
| `rocketchat_canaux`     | Canaux, groupes privés et messages directs de l'utilisateur                        | lecture  |
| `rocketchat_historique` | Messages d'un canal ou d'un fil (dates, pagination)                                | lecture  |
| `rocketchat_chercher`   | Recherche de messages dans un canal                                                | lecture  |
| `rocketchat_envoyer`    | Message dans un canal, un fil, ou à `@utilisateur` (message direct créé si besoin) | écriture |
| `rocketchat_reagir`     | Réaction emoji à un message                                                        | écriture |

### 5.5 Nombre d'outils

Il y a environ 25 outils en 0.2.0 et environ 60 une fois tout activé. Les interrupteurs par
service et pour l'écriture réduisent la liste à ce que l'utilisateur utilise vraiment. La
description du serveur (`instructions`) est complétée par un paragraphe par service activé.

## 6. Signature des écritures

Toute écriture **indique qu'elle vient d'Ocekour** :

- **Toujours** : en-tête `User-Agent: Ocekour/<version> (agent MCP)` sur chaque requête
  (visible dans les journaux des serveurs).
- **Texte ajouté** en fin de contenu, là où il y a du texte :

| Écriture                                                                               | Signature                                                                                                                                                    |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Commentaire, issue, MR (GitLab) · message Rocket.Chat                                  | dernière ligne `_— 🤖 via Ocekour_`                                                                                                                          |
| Commentaire, ticket, worklog (Jira) · commentaire (Confluence)                         | dernière ligne `_— via Ocekour (agent IA)_` : **sans emoji**, car les bases MySQL « utf8 » (3 octets) courantes sous Jira / Confluence DC refusent les emoji |
| Page Confluence créée                                                                  | libellé `ocekour` (l'API de création n'accepte pas de message de version)                                                                                    |
| Page Confluence modifiée                                                               | message de version « via Ocekour (agent IA) » **et** libellé `ocekour` ; le corps n'est pas modifié                                                          |
| Sans texte (transition, assignation, relance de pipeline, réaction, résolution de fil) | User-Agent et journal local uniquement                                                                                                                       |

La signature est fixe : ni l'agent ni la configuration ne peuvent la retirer.

## 7. Journal des actions d'écriture

- Fichier local `%APPDATA%\Ocekour\actions.ndjson`, une ligne par écriture **tentée** (réussie
  ou non). Rotation à 10 Mo (un fichier précédent conservé).
- Contenu de chaque entrée : date, **client MCP** (nom et version déclarés à la connexion, par
  ex. `claude-code 2.x`), service, outil, cible (projet/MR, ticket, page, canal), **URL web** du
  résultat, paramètres envoyés (texte complet, jamais de jeton), statut, message d'erreur.
- Page **« Actions de l'agent »** (menu principal) : liste la plus récente en premier, filtres
  par service, statut et texte, bouton « Ouvrir ↗ » vers l'élément, détail dépliable (paramètres)
  et bouton « Copier ». Une pastille signale les nouvelles actions depuis la dernière visite.
- Les **lectures** ne vont pas dans ce journal, seulement dans le journal technique habituel
  (outil + durée, sans contenu).

## 8. Matomo

Événements, sans contenu, sans URL de serveur ni nom de projet :

| Catégorie | Action                                                                                           | Nom                                                          |
| --------- | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| `MCP`     | nom de l'outil (ex. `gitlab_mr_commenter`, préfixé par le service), comme les outils de la 0.2.0 | `ok` / `erreur`                                              |
| `Comptes` | `jeton_cree_ouvert`, `jeton_enregistre`, `jeton_test`                                            | service (+ `ok` / `erreur` pour l'enregistrement et le test) |
| `Comptes` | `ecriture_activee` / `ecriture_desactivee`                                                       | service                                                      |

## 9. Clients documentés

Guide et aide intégrée (sélecteur de client) : **Claude Code**, **Qwen Code**,
**Mistral Vibe**, autres clients, mode sans interface. Pour Mistral Vibe
(`~/.vibe/config.toml`, section `[[mcp_servers]]`, transport HTTP + en-tête
`Authorization`) : vérifié avec le vrai client (voir §12).

## 10. Implémentation

```
electron/services/
  http.ts          client HTTP(S) commun : TLS (réglage Rancher), User-Agent, erreurs lisibles (401 → Comptes)
  definitions.ts   par service : libellé, page de création du jeton, en-têtes, test du jeton
  accounts.ts      config (`services`) + secrets + test + page de jeton ; événement « changé »
  journal.ts       actions.ndjson, rotation 10 Mo, lecture filtrée et paginée
  markup.ts        signature ; Markdown → wiki Jira ; Markdown ↔ stockage Confluence (marked, turndown)
  tools.ts         kit lecture / écriture : disponibilité, signature, journal ; visibilité des outils
  gitlab/          position.ts (lignes du diff, line_code, line_range), tools.ts
  jira/ confluence/ rocketchat/   tools.ts
electron/mcp/tools.ts   outils 0.2.0 + registerServiceTools ; Matomo ok / erreur par outil
shared/config.ts        ServicesSchema ; licence/preset.ts accepte `services`
src/app/settings/accounts.ts      écran Comptes
src/app/actions/actions-view.ts   page « Actions de l'agent » + pastille
```

- Tous les outils sont déclarés, puis **activés ou désactivés** selon le service (URL, jeton,
  interrupteur) et l'autorisation d'écrire. Le serveur HTTP crée un `McpServer` par requête :
  un changement prend effet dès l'appel suivant. En stdio, la liste est mise à jour et notifiée
  (`tools/list_changed`). Chaque appel revérifie aussi la disponibilité.
- Un wrapper commun à toutes les écritures : vérifier `write`, appeler l'API (texte signé),
  journaliser (succès ou échec), événement Matomo.

## 11. Tests

- **Faux serveurs** (sur le modèle de `fake-rancher`) pour chaque service : réponses réelles
  enregistrées, erreurs 401/403/404/409, pagination, calcul des positions de commentaire GitLab
  (ligne ajoutée, supprimée, inchangée, hors diff, plage).
- **Tests d'intégration facultatifs** sur vos instances, activés par variables d'environnement
  (`GITLAB_TEST_URL` + `GITLAB_TEST_TOKEN`, etc.), à lancer en local. Vous validez sur les vraies
  instances avant chaque release.
- Pas de GitLab, Jira ni Confluence réels en CI (trop lourds, ou soumis à licence).

## 12. Décisions prises à l'implémentation

1. **Écriture désactivée par défaut**, autorisée service par service (confirmé).
2. **Signature** : voir §6 (sans emoji côté Atlassian ; libellé + message de version pour
   les pages Confluence).
3. **Approuver / merger une MR** : exclus en 0.3.x.
4. **Journal** : texte complet des paramètres conservé (fichier local).
5. **Client MCP** du journal : nom déclaré à l'initialisation (stdio), sinon `User-Agent` HTTP
   (le serveur HTTP est sans session : l'appel d'outil n'est pas lié à l'initialisation).
6. **Mistral Vibe** vérifié avec le vrai client (2.25.8) : `vibe mcp add … --transport
streamable-http --header "Authorization=Bearer …"`, liste des outils et appel d'écriture
   (journalisé comme `MistralAI-VibeCLI/2.25.8`).
7. **GitLab < 15.7** : `mr_diff` repasse sur `/changes` si `/diffs` n'existe pas ; page de jeton
   « ancienne » proposée pour GitLab < 16.
