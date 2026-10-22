# Configuration

[← Sommaire](README.md)

## Écran Paramètres

| Réglage                          | Défaut  | Rôle                                                                                             |
| -------------------------------- | ------- | ------------------------------------------------------------------------------------------------ |
| Thème                            | Système | Système, Clair ou Sombre                                                                         |
| Lignes chargées à l'ouverture    | 1 000   | Valeur proposée par défaut dans **Lignes**                                                       |
| Lignes gardées par onglet        | 100 000 | Au-delà, les plus anciennes sont retirées de la mémoire                                          |
| Rouvrir les onglets au démarrage | oui     | Onglets, options et filtres de la session précédente                                             |
| Dossier des mises à jour         | —       | _Réservé à une prochaine version_ (détection automatique d'un nouvel exe sur un partage réseau)  |
| Licence                          | —       | Type et libellé de la clé d'activation, **Changer de clé**                                       |
| Serveur MCP                      | non     | Serveur MCP pour les agents, port, jeton, configuration des clients (voir [Serveur MCP](mcp.md)) |
| Journal                          | —       | **Afficher le journal** ouvre l'Explorateur sur `logs\main.log`                                  |

L'onglet **config.yaml** des Paramètres est un éditeur avec coloration et vérification en
direct : les erreurs sont soulignées, avec le numéro de ligne. **Ctrl+S** enregistre.
**Recharger depuis le disque** et **Afficher le fichier** sont aussi disponibles. Les
modifications faites dans un autre éditeur sont prises en compte à chaud.

Si le fichier devient invalide, Ocekour garde la **dernière configuration valide** et affiche
l'erreur.

## Le fichier `%APPDATA%\Ocekour\config.yaml`

Les **commentaires sont conservés** lorsque Ocekour modifie le fichier. Exemple complet :

```yaml
theme: system # system | light | dark
bufferSize: 100000
defaultTailLines: 1000
restoreTabs: true

connections:
  - id: rancher-1a2b3c4d # généré, ne pas modifier
    type: rancher
    name: Rancher prod
    url: https://rancher.intra
    tls:
      insecure: false
      caFile: C:\certs\ac-entreprise.cer # facultatif
    clusters:
      - { id: c-m-abcd1234, name: prod }

  - id: kafka-5e6f7a8b
    type: kafka
    name: Kafka prod
    brokers: [kafka1.intra:9092, kafka2.intra:9092]
    security:
      protocol: SASL_PLAINTEXT # SASL_PLAINTEXT | SASL_SSL | PLAINTEXT
      mechanism: SCRAM-SHA-512 # SCRAM-SHA-512 | SCRAM-SHA-256 | PLAIN
      username: mon-user
    registry: # facultatif (Avro)
      url: http://schema-registry.intra:8081

  - id: s3-9c0d1e2f
    type: s3
    name: S3 archives
    url: https://s3.intra
    region: eu-west-1 # facultatif
    pathStyle: true
    accessKeyId: AKIA...
    buckets: [archives-logs] # buckets ajoutés à la main (facultatif)

favorites:
  - rancher-1a2b3c4d/c-m-abcd1234/app-commandes
  - rancher-1a2b3c4d/c-m-abcd1234/app-commandes/api-commandes

profiles: [] # voir ci-dessous

services: # hub MCP (Paramètres → Comptes), voir hub-mcp.md
  gitlab:
    url: https://gitlab.intra
    tls: { insecure: false } # facultatif, comme pour Rancher
    enabled: true # outils visibles par les agents (défaut : oui)
    write: false # écritures autorisées (défaut : non)
  jira: { url: https://jira.intra }
  confluence: { url: https://confluence.intra }
  rocketchat: { url: https://chat.intra }

notifications: # Paramètres → Notifications, voir notifications.md
  enabled: true
  intervalSeconds: 60
  target: self # self | "#canal" | "@personne"
  desktop: true # aussi en notifications Windows
  sources:
    gitlab: { enabled: true }
    jira: { enabled: true }
    confluence: { enabled: true }

contexte: # Paramètres → Contexte IA, voir contexte-ia.md
  enabled: false # désactivé par défaut ; 1re activation = part de maintenant (pas de rattrapage)
  intervalSeconds: 60
  llm: auto # auto | qwen | vibe | off (off = texte brut, sans résumé)
  tls: { insecure: false } # endpoint LLM en https : mêmes réglages TLS que Rancher (caFile possible)
  sources:
    rocketchat: { enabled: true } # tout message reçu
    gitlab: { enabled: true }
    jira: { enabled: true }
    confluence: { enabled: true }
```

Les **secrets n'y figurent jamais** : ils sont dans `secrets.json`, chiffrés par la clé locale `secret.key`. Cela vaut aussi
pour les jetons GitLab, Jira, Confluence et Rocket.Chat.

## Profils

Un profil règle l'affichage d'un workload ou d'un topic. Les menus **⋯** du détail les
écrivent pour vous. Vous pouvez aussi les écrire à la main :

```yaml
profiles:
  # Réglages d'un workload précis : <cluster>/<namespace>/<workload>
  - match: prod/app-commandes/api-commandes
    fields: # remplace la détection automatique
      time: '@timestamp'
      level: log.level
      message: message
    columns: [traceId, http.status] # colonnes de la liste
    expand: [error.stack, sql] # champs dépliés en tête du détail
    wrap: ['@message', http.body] # JSON indenté dans ces colonnes (@message : colonne message)
    parseNested: true # JSON imbriqué dans des chaînes

  # Un topic Kafka : kafka/<nom de la connexion>/<topic>
  - match: kafka/Kafka prod/commandes.events
    columns: [orderId, status]

  # Jokers : * = un segment, ** = n'importe quoi
  - match: '*/*/batch-*'
    expand: [job.parameters]

  # Valeurs par défaut pour tout
  - match: '**'
    columns: [traceId]
```

**Tous** les profils qui correspondent s'appliquent, dans l'ordre du fichier :

- pour `fields` et `parseNested`, le **premier** qui les définit l'emporte : placez donc les
  profils précis avant les profils génériques ;
- `columns`, `expand` et `wrap` se **cumulent**.

Les chemins de champs utilisent le point (`error.stack`, `http.status`).

## Statistiques d'utilisation

Selon le type de votre [clé d'activation](installation.md#activation), Ocekour envoie des
statistiques d'utilisation **anonymes** au serveur Matomo de l'équipe. Elles servent à savoir
quelles fonctions sont utilisées et quelles erreurs reviennent. La destination est fournie
par la clé : elle n'apparaît ni dans `config.yaml`, ni dans l'application, ni dans le journal.

| Envoyé          | Exemples                                                                                                                                                                                                                            |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Écrans affichés | Accueil, Logs Rancher, Messages Kafka, Explorateur S3, Paramètres                                                                                                                                                                   |
| Actions         | Connexion créée (type), test réussi, flux ouvert (type, suivi/historique, nombre de lignes), lien copié ou ouvert, export (format, nombre de lignes), aperçu S3 (texte, image…), opérations S3 (nombre d'objets), réglage de profil |
| Erreurs         | Le canal et le **code** seulement (`unauthorized`, `network`…)                                                                                                                                                                      |
| Contexte        | Version d'Ocekour, identifiant de la clé, système (en-tête User-Agent), langue, résolution d'écran                                                                                                                                  |

**Jamais envoyé** : secrets, URL ou noms de serveurs et de connexions, clusters, pods,
topics, buckets, textes recherchés, contenu des logs, nom d'utilisateur ou de poste, libellé
de la clé. Le visiteur est un identifiant **aléatoire** généré sur le poste
(`%APPDATA%\Ocekour\statistiques-visiteur`). Une visite correspond à une session
d'utilisation d'Ocekour.
