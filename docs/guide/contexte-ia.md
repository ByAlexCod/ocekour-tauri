# Contexte IA : une mémoire pour vos agents

[← Sommaire](README.md)

Depuis la 0.7.0, Ocekour peut tenir une **mémoire de contexte** locale : chaque **message
Rocket.Chat de vos salons** (reçu **comme envoyé**) et chaque **nouveauté GitLab, Jira ou
Confluence** qui vous concerne y sont versés. Votre agent local (Qwen Code, Mistral Vibe) y accède par le **serveur
MCP** d'Ocekour : quand vous lui posez une question, il a tout ce contexte à portée de main.

> **Sans base de données, sans vecteurs.** Le contexte est un simple répertoire de fichiers
> Markdown (un par MR, ticket, page ou conversation), recherché en plein-texte. Tout reste sur
> le poste ; rien n'est envoyé ailleurs.

## Activer

**Paramètres → Contexte IA** → cochez _Nourrir une mémoire de contexte et l'exposer à mon
agent_. La fonction est **désactivée par défaut**.

- À la **première activation**, Ocekour part de « maintenant » : il n'y a **pas de rattrapage**
  de l'historique.
- **Intervalle** : toutes les 60 s par défaut (minimum 30 s).
- Chaque **source** (Rocket.Chat, GitLab, Jira, Confluence) se coche séparément et n'est
  surveillée que si elle a un **jeton** (Paramètres → Comptes).
- La mémoire ne part que **pendant qu'Ocekour tourne** (fenêtre ouverte, `--mcp-http` ou
  `--notify`).

## Ce qui est ingéré

| Source         | Ce qui alimente le contexte                                                       |
| -------------- | --------------------------------------------------------------------------------- |
| **Rocket.Chat**| **Tout message d'un salon**, reçu **comme envoyé** (messages directs et canaux dont vous êtes membre) |
| **GitLab**     | MR, commentaires et réponses, revues demandées, pipelines… (comme les notifications) |
| **Jira**       | Tickets vous concernant : commentaires, changements de statut, assignations        |
| **Confluence** | Pages que vous suivez : commentaires, modifications                                |

Sur Rocket.Chat, **vos propres messages sont aussi ingérés** (utile : l'agent connaît vos
décisions et demandes). Ailleurs (GitLab, Jira, Confluence), les actions que **vous** faites ne
créent pas d'entrée.

## Le résumé par un LLM (facultatif)

Ocekour peut **résumer** chaque nouveauté avec le **LLM déjà configuré** dans Qwen Code ou
Mistral Vibe — il lit leur configuration, **aucune clé n'est demandée ni stockée** par Ocekour.

- Réglage **Résumé par le LLM de** : _auto_ (Qwen puis Vibe), _Qwen Code_, _Mistral Vibe_, ou
  _Aucun_ (on garde le texte brut).
- Si aucun LLM n'est trouvé, Ocekour stocke le **texte brut** : le contexte reste utile.
- Le résumé n'est **jamais bloquant** : en cas d'erreur ou de lenteur, on garde le brut.
- Ocekour lit l'endpoint et le modèle dans `~/.qwen/settings.json` : la structure
  `modelProviders` + `security.auth.selectedType` + `model.name` (fournisseurs OpenAI-compatibles,
  dont **vLLM auto-hébergé**), la clé étant résolue via `envKey` dans le bloc `env`. Les formes
  simples (`model.baseUrl`) et `~/.qwen/.env` (`OPENAI_BASE_URL`…) sont acceptées en secours.
- **TLS de l'endpoint LLM** : si le LLM est en **https** avec un certificat auto-signé ou une CA
  d'entreprise, cochez _Ignorer le certificat_ ou indiquez un **certificat d'autorité**, comme
  pour les autres services (Rancher, Comptes).

## Brancher votre agent (bouton)

Sous **Configurer mon agent**, un bouton enregistre le serveur MCP d'Ocekour dans la
configuration de l'agent détecté :

- **Qwen Code** — ajoute `ocekour` à `mcpServers` de `~/.qwen/settings.json` ;
- **Mistral Vibe** — ajoute une entrée `[[mcp_servers]]` à `~/.vibe/config.toml`.

Le fichier existant est **sauvegardé** (`.bak`) et l'opération est **idempotente** (relancer
ne crée pas de doublon). Le bouton **Retirer** défait l'enregistrement. Le **serveur MCP**
d'Ocekour doit être activé (Paramètres → Serveur MCP).

Une fois branché, votre agent dispose de trois outils :

| Outil                          | Rôle                                                          |
| ------------------------------ | ------------------------------------------------------------- |
| `contexte_chercher(motscles)`  | Recherche plein-texte ; renvoie extraits et références        |
| `contexte_recents(limite?)`    | Les entités récemment mises à jour                            |
| `contexte_lire(ref)`           | Le contenu complet d'une entité (par référence ou par clé)    |

## Où est la mémoire

Dans `%APPDATA%\Ocekour\contexte\`, un fichier par entité :

```
contexte\
  index.md            ← sommaire des dernières entités
  gitlab\grp-app!42.md
  jira\PAY-123.md
  confluence\98765.md
  rocketchat\@amartin.md
```

Chaque fichier a un en-tête (titre, lien, participants) et un journal horodaté. **Sans purge** :
un fichier trop gros voit son ancien contenu replié dans `contexte\<source>\archive\` (rien
n'est perdu, la recherche couvre l'archive).

## Bon à savoir

- **Rien ne quitte le poste.** Le contexte n'est jamais envoyé à Matomo, ni dans un lien de
  partage. Le LLM de résumé vise un **modèle local** (Qwen OpenAI-compatible, ou Vibe configuré
  en interne).
- **Aucun secret** dans les fichiers : seulement titres, liens et extraits déjà visibles.
- **Indépendant des notifications** : le contexte peut être actif sans les notifications
  Rocket.Chat, et réciproquement.
- **Rocket.Chat** : Ocekour inspecte les salons ayant **bougé** depuis le dernier passage
  (message reçu ou envoyé) ou ayant du **non-lu**. Un salon lu ailleurs (téléphone…) sans
  nouveau message avant le passage peut être manqué.
