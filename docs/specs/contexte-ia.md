# Spec — Contexte IA : mémoire de contexte alimentée en continu (0.7.0)

> Statut : **validée, implémentée en 0.7.0** · Guide utilisateur : [contexte-ia.md](../guide/contexte-ia.md)
> · Suite du hub MCP ([spec](hub-mcp.md)) et des notifications ([spec](notifications.md))

## 1. Objectif

Ocekour tient à jour, **en local et sans base vectorielle**, une **mémoire de contexte**
alimentée en continu par tout ce qui concerne l'utilisateur :

- chaque **message Rocket.Chat, reçu comme envoyé** (DM ou canal où il est) ;
- chaque **nouveauté GitLab, Jira, Confluence** le concernant (les mêmes signaux que les
  [notifications](notifications.md)).

Cette mémoire est un **répertoire multi-fichiers** organisé selon les standards du domaine
(`AGENTS.md`, _memory bank_). Elle est **exposée aux agents locaux** (Qwen Code, Mistral Vibe)
**uniquement par le MCP** d'Ocekour : quand l'utilisateur pose une question à son agent, celui-ci
a « à portée de main » l'ensemble du contexte, via des outils `contexte_*`.

Deux principes structurants demandés :

1. **Pas de RAG vectoriel** pour le moment : recherche par **mots-clés / plein-texte / structure
   de fichiers**, jamais par similarité d'embeddings.
2. **Réutiliser le LLM déjà configuré** dans Qwen Code ou Vibe (lu dans leur config) pour la
   curation ; ne pas demander une nouvelle clé.

Ce qu'apporte cette version, en une phrase : Ocekour passe de **hub MCP** (il donne accès aux
services) à **mémoire ambiante** (il observe les services et en fait un contexte prêt à l'emploi).

Hors périmètre 0.7.x : base vectorielle / recherche sémantique ; ingestion d'autres sources
(courriel, calendrier) ; partage de la mémoire entre postes ; résumé multi-entités « projet ».

## 2. Le motif « ambient agent » avec LangGraph (JS)

Le moteur d'ingestion est un **graphe LangGraph** (`@langchain/langgraph`, **JS/TS**), embarqué
**dans le process Electron main** — pas de sidecar Python, le portable reste un seul `.exe`.

Le graphe n'est **pas** déclenché par un chat mais par des **événements** (motif _ambient agent_).
Deux origines alimentent le graphe :

- les sources déjà sondées par le `Notifier` (GitLab / Jira / Confluence) ;
- **un nouveau sondage Rocket.Chat « tout message, reçu comme envoyé »** (§7), propre à cette feature — il ne
  fait pas partie des notifications, qui ne prennent que « ce qui me concerne ».

```
événement (RC reçu / GitLab / Jira / Confluence)
        │
   [classer / router]      ← décide de l'entité concernée (MR, ticket, page, fil)
        │
   [charger le fichier d'entité]   (BaseStore = index anti-doublon + curseurs)
        │
   [curer]                 ← LLM local : résume/normalise l'ajout (facultatif, §5)
        │
   [écrire le fichier + rafraîchir index.md]
```

- **État du graphe** : les événements traversent les nœuds `classer → curer → écrire`. La
  déduplication et les curseurs sont tenus par le moteur (`ContextEngine`) dans son propre état
  `contexte-state.json` (curseurs GitLab/Jira/Confluence + curseurs par salon Rocket.Chat + fenêtre
  anti-doublon), indépendant de `notifications.json`.
- Le moteur a son **propre sondage** (défaut 60 s), **indépendant des notifications** : il
  **réutilise les mêmes fonctions de fetch** (GitLab/Jira/Confluence) mais avec ses propres
  curseurs, et un **fetch Rocket.Chat « tout message, reçu comme envoyé »** qui lui est propre. Décision assumée :
  quand les deux features sont actives sur une même source, il peut y avoir deux appels réseau (le
  découplage prime sur la mutualisation ; chaque appel — To-Do, JQL, CQL — reste léger).

> **Activation.** La feature entière s'active/se désactive dans l'interface (§6) et est
> **désactivée par défaut**. À la première activation, elle part de « maintenant » : **aucun
> rattrapage** de l'historique (comme les notifications au premier lancement). Désactivée, aucun
> sondage propre ne tourne et les outils MCP `contexte_*` ne sont pas exposés.

> LangGraph apporte ici la **structure** (nœuds, reprise, état transverse) et une base commune
> pour faire évoluer la curation (résumé, consolidation) sans réécrire la tuyauterie.

## 3. Le répertoire de contexte

### 3.1 Emplacement et forme

```
%APPDATA%\Ocekour\contexte\
  AGENTS.md              ← consigne : « interroge d'abord les outils contexte_* d'Ocekour »
  index.md               ← sommaire vivant : ce qui bouge, le plus récent en tête
  gitlab/
    grp-app!42.md        ← une MR   : titre, état, fil des commentaires, liens
    grp-app#17.md        ← un ticket
  jira/
    PAY-123.md
  confluence/
    98765.md             ← une page (id)
  rocketchat/
    @amartin.md          ← un correspondant (DM)
    canal-paiement.md    ← un canal
```

Choix retenu (recommandation validée) : **un fichier par entité** + un `index.md` global. C'est
la granularité qui reste lisible par un humain **et** par l'agent, et qui borne la taille de
chaque fichier. La forme s'inspire du _memory bank_ (fichiers Markdown auto-entretenus) mais
**orientée entités** plutôt que fichiers globaux.

### 3.2 Format d'un fichier d'entité

En-tête YAML (métadonnées stables) + corps Markdown (journal horodaté, plus récent en bas) :

```markdown
---
source: gitlab
type: merge_request
ref: grp/app!42
titre: Paiement en double
url: https://gitlab.intra/grp/app/-/merge_requests/42
etat: opened
maj: 2026-09-29T14:12:00+02:00
participants: [amartin, bob]
---

## 2026-09-29 14:12 — @amartin a commenté
> Le montant est appelé deux fois quand le paiement échoue puis réussit.
[voir](https://gitlab.intra/grp/app/-/merge_requests/42#note_9001)

## 2026-09-29 13:40 — révision demandée
```

- **Sans purge** (décision) : rien n'est supprimé. Un fichier grossit ; s'il dépasse un seuil,
  on **replie** l'ancien dans `…/archive/<ref>.md` en gardant l'en-tête et un lien (pas de perte).
- Écriture **atomique** (fichier temporaire + `rename`) ; jamais de secret dans ces fichiers,
  seulement titres, liens, extraits déjà visibles par l'utilisateur.

### 3.3 `index.md`

Table des dernières entités touchées (ref, source, titre, date, lien vers le fichier). C'est le
point d'entrée que l'outil `contexte_recents` renvoie.

## 4. Exposition aux agents — **tout passe par le MCP**

Décision : **aucun accès direct au disque** par l'agent. Le répertoire vit dans `%APPDATA%` et
n'est **pas** posé dans le dépôt de l'utilisateur. L'agent lit le contexte via des **outils MCP**
ajoutés au serveur existant (`/mcp`, `--mcp` stdio) :

| Outil                         | Rôle                                                                 |
| ----------------------------- | -------------------------------------------------------------------- |
| `contexte_chercher(motscles)` | Recherche plein-texte sur tout le répertoire ; renvoie extraits+refs |
| `contexte_recents(limite?)`   | Le contenu d'`index.md` : ce qui a bougé récemment                   |
| `contexte_lire(ref)`          | Le fichier d'une entité (par `ref` ou chemin renvoyé par les autres) |

- **Recherche sans vecteurs** : indexation plein-texte incrémentale en mémoire (mots → fichiers),
  reconstruite au démarrage, mise à jour à chaque écriture. Tri par récence + nombre d'occurrences.
- Ces outils sont **en lecture seule** et **hors du chemin d'écriture** des agents : ils n'entrent
  pas dans « autoriser l'écriture » ni dans le journal des actions (comme les notifications).
- Un interrupteur `services.contexte` (comme les autres services) rend le groupe d'outils visible
  ou non.

L'`AGENTS.md` du répertoire n'est pas lu par l'agent (tout passe par le MCP) ; il sert de
**consigne optionnelle** que le bouton d'auto-config (§6) peut copier dans le projet de
l'utilisateur pour dire à l'agent de préférer `contexte_*`.

## 5. LLM de curation : lu dans la config de Qwen / Vibe

La curation (nœud `[curer]`) réutilise le **LLM déjà configuré** par l'utilisateur. Ocekour lit
leur configuration ; **aucune clé n'est demandée ni stockée par Ocekour** (elle reste chez l'outil
tiers).

| Outil          | Fichiers lus                                    | Clés / champs                                                                  |
| -------------- | ----------------------------------------------- | ------------------------------------------------------------------------------ |
| **Qwen Code**  | `~/.qwen/settings.json`, `.qwen/settings.json`  | `model.name`, `model.baseUrl` ; sinon env `OPENAI_MODEL/OPENAI_BASE_URL/OPENAI_API_KEY` |
| **Mistral Vibe** | `~/.vibe/config.toml`, `~/.vibe/.env`         | `active_model` ; clé `MISTRAL_API_KEY` (dans `.env`)                           |

- Endpoint **OpenAI-compatible** (`/v1/chat/completions`) : couvre Qwen (OpenAI-compat) et un
  Ollama/LM Studio/vLLM local pointé par la même config. Vibe est ici **toujours configuré vers un
  modèle interne** (endpoint local) : la curation reste donc sur le poste.
- **Détection** : Ocekour cherche Qwen puis Vibe ; l'utilisateur peut forcer l'un ou l'autre dans
  l'écran (§6). Aucun trouvé → **mode dégradé** : on stocke **brut, sans résumé** (le contexte
  reste utile ; la curation s'active dès qu'une config est présente).
- Prompt de curation borné (extrait + consigne « normalise en 1-3 lignes, garde les liens, pas
  d'invention »). Échec/temps dépassé → on retombe sur le brut. La curation n'est **jamais**
  bloquante pour l'ingestion.

## 6. Auto-configuration du MCP (bouton dans l'interface)

L'écran **Paramètres → Contexte IA** porte l'**interrupteur maître** de la feature (désactivée par
défaut, §2). Puisqu'Ocekour sait **lire** la config de Qwen/Vibe, il sait aussi y **écrire** : un
bouton **« Configurer mon agent »** par outil détecté enregistre le MCP d'Ocekour.

Ce que le bouton écrit, en préservant le reste du fichier (fusion, pas d'écrasement) :

**Qwen Code** — dans `~/.qwen/settings.json`, clé `mcpServers` :

```json
{
  "mcpServers": {
    "ocekour": {
      "httpUrl": "http://127.0.0.1:<port>/mcp",
      "headers": {
        "Authorization": "Bearer <jeton MCP>",
        "Accept": "application/json, text/event-stream"
      }
    }
  }
}
```

**Mistral Vibe** — dans `~/.vibe/config.toml`, entrée `[[mcp_servers]]` :

```toml
[[mcp_servers]]
name = "ocekour"
transport = "streamable-http"
url = "http://127.0.0.1:<port>/mcp"
  [mcp_servers.auth]
  type = "static"
  # en-tête Authorization: Bearer <jeton MCP>
```

- Le `<port>` et le `<jeton MCP>` sont ceux du serveur MCP d'Ocekour (Paramètres → Serveur MCP) ;
  le serveur doit être **activé** (le bouton le propose sinon).
- **Sauvegarde** du fichier existant (`settings.json.bak` / `config.toml.bak`) avant écriture ;
  fusion idempotente (relancer le bouton ne duplique pas l'entrée).
- Bouton **« Retirer »** symétrique (enlève l'entrée `ocekour`).
- Option : **copier l'`AGENTS.md`** de consigne dans le projet ouvert (chemin choisi par
  l'utilisateur), pour orienter l'agent vers `contexte_*`.
- Confirmation avant d'écrire dans un fichier hors d'Ocekour (action explicite, hors périmètre du
  journal des actions d'écriture des agents).

## 7. Ingestion Rocket.Chat — **tout message, reçu comme envoyé**

Nouveau, au-delà des notifications (qui ne prenaient que « ce qui me concerne ») : la mémoire
ingère **tout message d'un salon de l'utilisateur** (DM et canaux dont il est membre), **reçu
comme envoyé** — ses propres messages sont inclus (l'agent connaît ainsi ses décisions/demandes).

- API Rocket.Chat : `subscriptions.get` (salons + `unread`, `lm`/`_updatedAt`), puis pour chaque
  salon **ayant bougé depuis le curseur** (message reçu ou envoyé) ou ayant du non-lu :
  `channels.history` / `im.history` / `groups.history` selon le type, depuis le curseur du salon.
  Se fonder sur `lm`/`_updatedAt` (et non le seul `unread`) est nécessaire pour capter les
  messages **envoyés** (qui ne créent pas de non-lu).
- **Curseur par salon** dans l'état (`rooms: { [rid]: since }`), même logique anti-doublon que
  les autres sources.
- Un message → append dans `rocketchat/<@correspondant|canal>.md` (auteur — « Vous » pour soi —,
  horodatage, extrait, lien permalien). Seuls les **messages système** (arrivée/départ…) sont
  ignorés.
- **Volume** : bornage par passage (N salons × M messages) ; les salons très actifs sont tronqués
  avec une note « … » — la mémoire n'est pas une archive exhaustive, c'est un contexte récent.
- Réglage `sources.rocketchat` (comme gitlab/jira/confluence) : on peut couper l'ingestion RC
  sans couper les notifications, et réciproquement.

## 8. Configuration (`config.yaml`)

```yaml
contexte: # Paramètres → Contexte IA
  enabled: false # désactivée par défaut ; à la 1re activation, part de « maintenant » (pas de rattrapage)
  llm: auto # auto | qwen | vibe | off  (off = stockage brut)
  sources:
    rocketchat: { enabled: true } # ingère tout message, reçu comme envoyé
    gitlab: { enabled: true }
    jira: { enabled: true }
    confluence: { enabled: true }
```

- Partage `intervalSeconds` et les jetons avec les notifications (mêmes comptes, §hub-mcp).
- `services.contexte.enabled` (interrupteur d'outils MCP) reste distinct de l'ingestion : on peut
  ingérer sans exposer, ou l'inverse.
- **Secrets** : aucun nouveau. Rien de nouveau dans `config.yaml` non plus (URLs déjà présentes).

## 9. Découpage

Tout en **0.7.0** (comme les versions précédentes, pas de sous-découpage) :

1. Répertoire de contexte + format + écriture atomique + index plein-texte (§3, §4).
2. Graphe LangGraph JS d'ingestion, branché sur le sondage partagé du `Notifier` (§2).
3. Ingestion Rocket.Chat « tout message, reçu comme envoyé » + curseurs par salon (§7).
4. Outils MCP `contexte_chercher/recents/lire` + interrupteur (§4).
5. Lecture config Qwen/Vibe pour le LLM de curation + mode dégradé (§5).
6. Écran **Paramètres → Contexte IA** + bouton d'auto-config MCP (lecture/écriture) (§6).
7. Tests (électron + e2e), guide utilisateur, version, CI.

## 10. Risques et décisions ouvertes

- **Fusion des fichiers de config tiers** : formats TOML (Vibe) et JSON avec commentaires possibles
  (Qwen). Prévoir un parseur tolérant + sauvegarde ; en cas de doute, **ne pas écrire** et afficher
  le bloc à coller à la main.
- **Endpoint LLM injoignable / lent** : jamais bloquant (mode brut), temps limité par appel.
- **Volume Rocket.Chat** : bornage strict par passage ; à surveiller sur les gros canaux.
- **Vie privée** : le contexte peut contenir des messages de tiers → il reste **strictement local**,
  jamais envoyé à Matomo, jamais dans un lien de partage, jamais renvoyé par le MCP au-delà des
  outils `contexte_*` que l'utilisateur a activés. La curation vise un **modèle local** (Qwen
  OpenAI-compat ou Vibe configuré en interne) : les extraits ne quittent pas le poste.
