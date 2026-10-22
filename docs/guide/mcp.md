# Serveur MCP : Ocekour pour les agents

[← Sommaire](README.md)

Ocekour intègre un **serveur MCP** (_Model Context Protocol_). Un agent comme Claude Code,
Qwen Code, Mistral Vibe ou tout client MCP peut ainsi faire **les mêmes actions que
l'interface** : lister les connexions, parcourir Rancher, lire des logs ou des messages
Kafka, explorer un bucket S3, créer un lien de partage…

Depuis la 0.3.0, le même serveur donne aussi accès à **GitLab, Jira, Confluence et
Rocket.Chat** avec vos jetons : voir [Hub MCP](hub-mcp.md).

## Activer

**Paramètres → Serveur MCP** → cochez _Activer le serveur MCP pendant que l'interface est
ouverte_.

- Le serveur écoute **uniquement ce poste** : `http://127.0.0.1:47110/mcp`. Le port se
  change dans les Paramètres.
- Chaque appel doit porter le **jeton** affiché (`Authorization: Bearer …`). **Nouveau jeton**
  invalide l'ancien.
- **Configurer un client MCP** donne la configuration toute prête, avec votre jeton, pour
  chaque client : choisissez-le dans la liste. Les formats **diffèrent d'un client à
  l'autre**.

## Configurer votre client

Remplacez `<jeton>` par celui des Paramètres. Si vous avez changé le port, adaptez aussi
`47110`.

### Claude Code

```bash
claude mcp add --transport http ocekour http://127.0.0.1:47110/mcp --header "Authorization: Bearer <jeton>"
```

Pour vérifier : `claude mcp list`.

### Qwen Code

Qwen Code, dérivé de Gemini CLI, n'utilise **ni la même option d'en-tête ni la même clé de
configuration** que Claude Code.

```bash
qwen mcp add --transport http ocekour http://127.0.0.1:47110/mcp -H "Authorization: Bearer <jeton>"
```

- La commande écrit dans `%USERPROFILE%\.qwen\settings.json` (portée utilisateur, par
  défaut). Avec `-s project`, elle écrit dans `.qwen\settings.json` du projet courant.
- Pour vérifier : `qwen mcp list`.

Ou directement dans `settings.json` :

```json
{
  "mcpServers": {
    "ocekour": {
      "httpUrl": "http://127.0.0.1:47110/mcp",
      "headers": { "Authorization": "Bearer <jeton>" }
    }
  }
}
```

> ⚠️ Qwen Code attend **`httpUrl`**. La clé `url` y désigne l'ancien transport SSE, que
> Ocekour ne propose pas : avec `url`, la connexion échoue.

### Mistral Vibe

```bash
vibe mcp add ocekour --url http://127.0.0.1:47110/mcp --transport streamable-http --header "Authorization=Bearer <jeton>"
```

- Notez `--header "Nom=valeur"`, avec un **`=`** et non `:` comme pour Claude Code.
- La commande écrit dans `%USERPROFILE%\.vibe\config.toml`. Dans Vibe, les outils s'appellent
  `ocekour_<outil>`, par exemple `ocekour_logs_lire`.

Ou directement dans `config.toml` :

```toml
[[mcp_servers]]
name = "ocekour"
transport = "streamable-http"
url = "http://127.0.0.1:47110/mcp"

[mcp_servers.auth]
type = "static"
headers = { "Authorization" = "Bearer <jeton>" }
```

### Autres clients (Cursor, VS Code…)

Format `mcpServers` le plus courant :

```json
{
  "mcpServers": {
    "ocekour": {
      "type": "http",
      "url": "http://127.0.0.1:47110/mcp",
      "headers": { "Authorization": "Bearer <jeton>" }
    }
  }
}
```

Les clients dérivés de Gemini CLI attendent `httpUrl`, comme Qwen Code.

## Sans interface

Ocekour peut tourner **sans fenêtre**, avec seulement le serveur MCP :

| Commande                           | Usage                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Ocekour.exe --mcp-http` | **Recommandé.** Serveur HTTP en arrière-plan, même adresse et même jeton que ci-dessus. Port : celui des Paramètres, ou `--mcp-port=47200`. À lancer par un raccourci ou une tâche planifiée                                                                                                                                                                                                                              |
| `Ocekour.exe --mcp`      | Le client lance Ocekour lui-même et dialogue par l'entrée et la sortie standard (stdio), sans jeton. Configuration : `{"mcpServers": {"ocekour": {"command": "C:\\…\\Ocekour.exe", "args": ["--mcp"]}}}`, identique pour Claude Code, Qwen Code et Claude Desktop. Pour Mistral Vibe : `[[mcp_servers]]` avec `transport = "stdio"`, `command` et `args = ["--mcp"]`. Ocekour s'arrête quand le client se ferme |

Le mode sans interface peut tourner **en même temps que l'interface**. Il utilise la même
configuration, les mêmes connexions et la même clé d'activation.

Si le mode sans interface tourne déjà, **ne cochez pas** la case du serveur MCP dans
l'interface : les deux utiliseraient le même port. Les Paramètres affichent alors « Port …
indisponible ». Vos clients restent servis par le mode sans interface.

> Mode `--mcp` (stdio) : l'exe répond immédiatement (aucune fenêtre, WebView2 non chargé).
> `ocekour-headless.exe --mcp` fait la même chose avec un exe encore plus léger.

## Outils

| Outil                                                              | Action                                                                                                                                                              |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `licence_etat`, `licence_activer`                                  | État de l'activation ; activer avec une clé (préconfiguration comprise)                                                                                             |
| `connexions_lister`                                                | Connexions, **sans aucun secret** (indique seulement si un secret est enregistré)                                                                                   |
| `connexion_tester`, `connexion_enregistrer`, `connexion_supprimer` | Tester, créer ou modifier (secret chiffré à part), supprimer                                                                                                        |
| `sources_parcourir`                                                | Namespaces, pods et conteneurs Rancher ; topics Kafka ; buckets S3                                                                                                  |
| `logs_lire`                                                        | Logs d'un conteneur ou messages d'un topic (N dernières lignes, depuis, début, suivi quelques secondes), analysés comme dans l'interface, avec recherche et niveaux |
| `logs_exporter`                                                    | Idem, écrit dans un fichier NDJSON ou texte                                                                                                                         |
| `profils_lister`, `profil_modifier`                                | Profils d'affichage (colonnes, champs, JSON indenté…)                                                                                                               |
| `s3_lister`, `s3_apercu`                                           | Contenu d'un dossier, début d'un fichier                                                                                                                            |
| `s3_telecharger`, `s3_envoyer`, `s3_renommer`, `s3_supprimer`      | Opérations sur les fichiers                                                                                                                                         |
| `lien_creer`, `lien_ouvrir`                                        | Liens de partage `ocekour://…`                                                                                                                                      |
| `configuration_lire`                                               | Contenu de `config.yaml`                                                                                                                                            |
| `attendre`                                                         | Attend un délai (secondes/minutes, 300 s au plus) : pour espacer des relectures de logs ou une boucle. L'agent peut couper au-delà de son propre délai (~60 s)      |
| `gitlab_*`, `jira_*`, `confluence_*`, `rocketchat_*`               | Si le compte est configuré : voir [Hub MCP](hub-mcp.md)                                                                                                             |

## Sécurité

- **Mêmes règles que l'interface.** Sans clé d'activation, seuls les outils de licence
  répondent. Avec une clé à suivi obligatoire, Matomo doit être joignable.
- **Aucun secret** (token, mot de passe, clé, jeton GitLab / Jira…) n'est jamais renvoyé.
- **Écritures dans GitLab, Jira, Confluence, Rocket.Chat** : seulement si vous les avez
  autorisées (Paramètres → Comptes). Elles sont signées « via Ocekour » et listées dans
  **Actions de l'agent**.
- **Actions destructrices.** `connexion_supprimer` et `s3_supprimer` exigent `confirmer: true`
  et sont signalées comme telles au client, qui demande en général votre accord.
- **Accès limité à ce poste.** Le serveur HTTP n'accepte que ce poste, avec le jeton, et
  refuse les autres noms d'hôte (protection contre le _DNS rebinding_).
