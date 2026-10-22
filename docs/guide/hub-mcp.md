# Hub MCP : GitLab, Jira, Confluence, Rocket.Chat

[← Sommaire](README.md)

Depuis la 0.3.0, le [serveur MCP](mcp.md) d'Ocekour donne aussi à vos agents (Claude Code,
Qwen Code, Mistral Vibe…) l'accès à **GitLab**, **Jira**, **Confluence** et **Rocket.Chat**.
Un seul serveur MCP à configurer suffit pour les logs et pour tous ces outils.

- L'agent agit **avec vos droits**, grâce à un **jeton que vous créez vous-même** sur chaque
  outil.
- Les jetons sont **chiffrés sur votre poste** (Windows). Ils ne sont jamais écrits dans
  `config.yaml` et jamais transmis aux agents.
- Toute écriture faite par un agent est **signée « via Ocekour »** et **inscrite dans un
  journal local** : la page [Actions de l'agent](#actions-de-lagent).

## Configurer un compte

**Paramètres → Comptes**. Il y a une carte par outil.

1. **URL** : l'adresse de votre instance, par ex. `https://gitlab.exemple.fr`, puis
   **Enregistrer**. Elle est souvent déjà remplie par votre clé d'activation. Pour un
   certificat d'entreprise, dépliez **Certificat (TLS)**. Le réglage est le même que pour
   Rancher.
2. **Créer un jeton ↗** ouvre la bonne page de l'outil dans votre navigateur. Pour GitLab,
   elle est déjà pré-remplie. Le lien **Comment créer le jeton ?** donne les étapes (voir
   aussi ci-dessous).
3. **Coller le jeton**, puis **Enregistrer**. Ocekour le teste aussitôt et affiche votre
   identité (`✓ Alex Martin (@amartin)`), ou l'erreur.
4. **Agents** :
   - _Outils … visibles par les agents_ : décochez pour masquer l'outil sans oublier le
     jeton.
   - _Autoriser l'écriture_ : **désactivée par défaut**. Tant qu'elle n'est pas cochée,
     l'agent ne voit que les outils de lecture. Il n'y a **pas de confirmation à chaque
     écriture** : votre client MCP (Claude Code, etc.) demande déjà votre accord avant
     d'utiliser un outil.

Les changements s'appliquent **immédiatement**, sans redémarrer ni reconfigurer le client
MCP.

### Créer le jeton, outil par outil

| Outil           | Page ouverte                                                                                         | À faire                                                                                                                                                                 |
| --------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **GitLab**      | _Préférences → Jetons d'accès personnels_, avec le nom `Ocekour` et la portée **`api`** déjà remplis | Choisir une date d'expiration → _Create personal access token_ → copier le jeton (affiché **une seule fois**). Avant GitLab 16, utilisez le bouton **ancienne page ↗**  |
| **Jira**        | _Profil → Jetons d'accès personnels_                                                                 | _Créer un jeton_ → nom `Ocekour` → expiration → _Créer_ → copier                                                                                                        |
| **Confluence**  | _Profil → Paramètres → Jetons d'accès personnels_                                                    | Idem Jira                                                                                                                                                               |
| **Rocket.Chat** | _Mon compte → Jetons d'accès personnels_                                                             | Nom `Ocekour`, **ne pas** cocher _Ignorer l'authentification à deux facteurs_ → _Ajouter_ → copier **le jeton et l'identifiant utilisateur** (deux champs dans Ocekour) |

- **GitLab** : la portée `api` est nécessaire pour écrire. Avec `read_api` seulement, la
  lecture fonctionne, et Ocekour signale la portée manquante.
- **Rocket.Chat** : si la section _Jetons d'accès personnels_ n'existe pas, demandez à
  l'administrateur la permission `create-personal-access-tokens`.
- **Jeton expiré** : l'agent reçoit « Jeton … refusé (expiré, révoqué ou invalide) :
  Ocekour → Paramètres → Comptes ». Créez-en un nouveau et remplacez-le.

## Ce que les agents peuvent faire

Les projets, tickets et pages se désignent comme dans le navigateur : `groupe/projet`,
`PAY-123`, ou **l'URL web collée telle quelle**. Chaque résultat contient le lien web de
l'élément.

### GitLab

| Lecture                                                                         | Écriture (si autorisée)                                                                              |
| ------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `gitlab_projets_chercher`, `gitlab_projet`                                      | `gitlab_mr_commenter` : commentaire **global**, ou **positionné sur une ligne ou une plage** du diff |
| `gitlab_arborescence`, `gitlab_fichier_lire`, `gitlab_code_chercher`            | `gitlab_discussion_repondre`, `gitlab_discussion_resoudre`                                           |
| `gitlab_commits`, `gitlab_commit_lire`, `gitlab_branches`                       | `gitlab_mr_creer`                                                                                    |
| `gitlab_mr_lister`, `gitlab_mr_lire`, `gitlab_mr_diff`, `gitlab_mr_discussions` | `gitlab_issue_creer`, `gitlab_issue_commenter`                                                       |
| `gitlab_issues_lister`, `gitlab_issue_lire`                                     | `gitlab_pipeline_relancer`, `gitlab_job_relancer`                                                    |
| `gitlab_pipelines_lister`, `gitlab_pipeline_lire`, `gitlab_job_log`             |                                                                                                      |

**Commentaire positionné** : l'agent indique le fichier, la ligne et le côté. Le côté
`nouveau` désigne une ligne ajoutée ou inchangée, numérotée dans la nouvelle version.
Le côté `ancien` désigne une ligne supprimée. Une _ligne de fin_ commente une plage. Ocekour
calcule lui-même la position GitLab. Si la ligne n'est pas dans les modifications de la MR,
l'agent reçoit la liste des lignes commentables au lieu d'un commentaire mal placé.

Approuver et merger une MR ne sont **pas** proposés.

### Jira

| Lecture                                                                            | Écriture (si autorisée)                               |
| ---------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `jira_chercher` (JQL)                                                              | `jira_ticket_creer`, `jira_ticket_modifier`           |
| `jira_ticket_lire` : champs personnalisés nommés, commentaires, liens, sous-tâches | `jira_commenter`                                      |
| `jira_projets` : projets, types, champs obligatoires                               | `jira_transition` : changement de statut, par nom     |
| `jira_transitions`, `jira_utilisateurs`                                            | `jira_assigner` (`moi`, `personne` ou un identifiant) |
| `jira_tableaux`, `jira_sprints`, `jira_sprint_tickets`                             | `jira_temps_saisir` (temps passé)                     |

L'agent écrit en **Markdown**, qu'Ocekour convertit au format de Jira. Il peut aussi envoyer
du texte Jira tel quel avec `format: wiki`.

### Confluence

| Lecture                                                  | Écriture (si autorisée)                                                                                     |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `confluence_chercher` : CQL, ou texte simple             | `confluence_page_creer`                                                                                     |
| `confluence_page_lire` : page convertie en **Markdown**  | `confluence_page_modifier` : exige la **version lue**, et refuse si quelqu'un a modifié la page entre-temps |
| `confluence_espace` : espaces, pages racines, sous-pages | `confluence_commenter`                                                                                      |
| `confluence_pieces_jointes` (contenu des fichiers texte) |                                                                                                             |

### Rocket.Chat

| Lecture                                      | Écriture (si autorisée)                                                  |
| -------------------------------------------- | ------------------------------------------------------------------------ |
| `rocketchat_canaux` : vos salons, non-lus    | `rocketchat_envoyer` : dans un canal, un fil, ou en direct (`@collegue`) |
| `rocketchat_historique` : un salon ou un fil | `rocketchat_reagir` : réaction emoji                                     |
| `rocketchat_chercher`                        |                                                                          |

## Signature des écritures

| Écriture                                                               | Signature                                                                                         |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Commentaire, issue, MR (GitLab) · message (Rocket.Chat)                | dernière ligne _— 🤖 via Ocekour_                                                                 |
| Commentaire, ticket, temps passé (Jira) · commentaire (Confluence)     | dernière ligne _— via Ocekour (agent IA)_                                                         |
| Page Confluence créée ou modifiée                                      | libellé `ocekour` et message de version « via Ocekour (agent IA) » ; le contenu n'est pas modifié |
| Sans texte (statut, assignation, relance, réaction, résolution de fil) | pas de signature visible : voir le journal                                                        |

En plus, chaque requête s'identifie auprès du serveur comme `Ocekour/<version> (agent MCP)`,
ce qui la rend visible dans les journaux de GitLab, Jira, etc. Ni l'agent ni la
configuration ne peuvent retirer la signature.

## Actions de l'agent

La page **Actions de l'agent** (menu du haut) liste **toutes les écritures tentées** par un
agent, réussies ou non, la plus récente en premier. Une pastille compte les nouvelles
actions depuis votre dernière visite.

- Pour chaque action : date, outil, élément visé, **client MCP** (par ex. `claude-code`,
  `MistralAI-VibeCLI`), bouton **Ouvrir ↗** vers le commentaire, ticket ou page, et les
  paramètres complets (**Copier**).
- Filtres par outil, statut et texte.
- Le journal reste sur votre poste : `%APPDATA%\Ocekour\actions.ndjson`, au plus 10 Mo, plus
  le fichier précédent.
- Les **lectures** n'y figurent pas. Seul le journal technique (`logs\main.log`) les trace,
  sans leur contenu.

## Clé d'activation

La clé de votre équipe peut **pré-remplir les URL** : section `services` de la
préconfiguration, voir le [guide administrateur](../cles-activation.md). Un outil que vous
avez déjà configuré n'est pas modifié. Vous pouvez toujours changer l'URL. La clé ne contient
jamais de jeton et n'active jamais l'écriture à votre place.
