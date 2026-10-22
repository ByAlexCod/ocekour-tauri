# Ocekour — Cadrage technico-fonctionnel

Visionneuse de logs locale pour Windows 10/11. Principe directeur : **KISS**, librairies éprouvées, aucune BDD, tout est local sauf les logs.

## 1. Décisions

| Sujet           | Décision                                                                                                                                                                                                                                                                               |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OS              | Windows 10 / 11 uniquement                                                                                                                                                                                                                                                             |
| Utilisateur     | Mono-utilisateur. Partage par copier/coller d'un « lien » texte (voir §6)                                                                                                                                                                                                              |
| Coquille        | **Electron** (backend Node.js / TypeScript) — pas de Rust                                                                                                                                                                                                                              |
| Front           | **Angular** (dernière version dispo sur le Nexus), standalone + signals, **Tailwind CSS**, thème **dark / light** (suit l'OS, bascule manuelle)                                                                                                                                        |
| Distribution    | **Un seul `.exe` portable** (electron-builder, cible `portable`) : on le dépose où on veut et on double-clique. Pas d'installeur, pas de droits admin. Non signé (avertissement SmartScreen au 1er lancement, accepté). Démarrage de quelques secondes (auto-extraction dans `%TEMP%`) |
| Mise à jour     | Optionnelle via **partage réseau** : au démarrage, Ocekour regarde s'il existe sur le partage un `Ocekour-<version>.exe` plus récent → bandeau « Nouvelle version » qui ouvre le dossier. Partage absent = rien ne se passe                                                            |
| TLS             | Par connexion : les autorités du **magasin Windows** sont reconnues automatiquement ; sinon case « ignorer la vérification SSL » **ou** choix d'un fichier `.cer`/`.crt`/`.pem` (DER ou PEM, converti automatiquement)                                                                 |
| Source v1       | **Rancher 2.5.5** : URL + **token**, Ocekour liste les clusters (`/v3/clusters`), on coche ceux voulus ; accès aux logs via l'API Kubernetes proxifiée par Rancher. Import d'un kubeconfig en alternative                                                                              |
| Portée v1       | Un pod / conteneur à la fois, option `previous`, mode **suivi temps réel** et **historique** (N dernières lignes ou depuis X min)                                                                                                                                                      |
| Source suivante | **SSH** (`ssh2`), puis fichiers locaux                                                                                                                                                                                                                                                 |
| Formats         | Ligne = JSON complet **ou** texte brut (pas de mixte). Chaînes JSON avec `\n`, `\t`… rendues proprement. JSON-dans-une-chaîne désérialisé (option)                                                                                                                                     |
| Configuration   | Fichier **YAML** local, éditable depuis l'interface, **profils par workload**                                                                                                                                                                                                          |
| Persistance     | Configuration uniquement. Export manuel NDJSON / texte                                                                                                                                                                                                                                 |
| Secrets         | Token chiffré via `safeStorage` d'Electron (DPAPI Windows), jamais en clair dans le YAML                                                                                                                                                                                               |
| Build           | Le `.exe` est produit **hors réseau air-gap** (CI GitHub Actions ou poste connecté) puis déposé. Aucun build dans l'air-gap                                                                                                                                                            |
| Langue UI       | Français                                                                                                                                                                                                                                                                               |

## 2. Architecture

```
┌──────────────────── Electron ─────────────────────┐
│ Main (Node)                                       │
│  ├─ sources/                                      │
│  │   ├─ LogSource (interface commune)             │
│  │   ├─ rancher.ts   (@kubernetes/client-node)    │
│  │   └─ ssh.ts       (ssh2)            [v2]       │
│  ├─ config.ts   (yaml + zod, fichier userData)    │
│  ├─ secrets.ts  (safeStorage)                     │
│  └─ updater.ts  (scan du partage réseau)          │
│           │ IPC typé (preload, contextIsolation)  │
│           │ lignes envoyées par lots (~100 ms)    │
│ Renderer (Angular + Tailwind)                     │
│  ├─ parser  : ligne brute → LogEvent              │
│  ├─ onglets : 1 flux = 1 onglet                   │
│  ├─ liste virtualisée (@angular/cdk/scrolling)    │
│  ├─ panneau détail (JSON déplié, \n rendus)       │
│  └─ éditeur YAML (CodeMirror 6)                   │
└───────────────────────────────────────────────────┘
```

Interface commune des sources (c'est ce qui permet d'ajouter SSH / local sans toucher au reste) :

```ts
interface LogSource {
  browse(path: string[]): Promise<Node[]>; // arborescence : cluster › namespace › pod › conteneur
  open(
    target: Target,
    opts: { follow: boolean; tail?: number; sinceSeconds?: number; previous?: boolean },
    onLines: (lines: string[]) => void,
  ): Stop;
}
```

Le parsing se fait côté front et ne dépend pas de la source.

## 3. Lecture « intelligente »

1. **Détection** : la ligne commence par `{` et `JSON.parse` réussit → JSON, sinon texte. L'horodatage Kubernetes (`timestamps=true`) sert de repli quand la ligne n'en a pas.
2. **Champs clés** auto-détectés (surchargeables par profil) :
   - heure : `@timestamp`, `timestamp`, `time`, `ts` ;
   - niveau : `level`, `severity`, `log.level`, `lvl` ;
   - message : `message`, `msg`.
3. **Vue liste** (hauteur de ligne fixe) : heure · niveau coloré · message sur une ligne (`\n` affichés `⏎`) + colonnes choisies du profil. Surlignage du terme filtré.
4. **Un clic → panneau détail** :
   - arbre JSON ;
   - toute chaîne multi-ligne est affichée **sur plusieurs lignes** (`white-space: pre`) ;
   - les champs `expand` du profil sont dépliés et mis en avant ;
   - si l'option `parseNested` est active, une chaîne qui contient du JSON est parsée et affichée en sous-arbre ;
   - boutons « copier la valeur » et « copier le JSON formaté ».
5. **Config en un clic** : menu « ⋯ » sur un champ du détail : « Toujours déplier en tête », « Afficher en colonne », « Utiliser comme heure / niveau / message » ; « Toujours pour ce workload » pour le JSON imbriqué. Le profil exact `cluster/namespace/workload` est créé ou complété en tête de `profiles`, commentaires conservés, et appliqué immédiatement (lignes déjà affichées ré-analysées).
6. **Filtres v1** : texte (sensible ou non à la casse, regex optionnelle) + niveaux (cases à cocher). Pause/reprise du suivi, défilement auto.
7. **Mémoire** : tampon circulaire de 100 000 lignes par onglet (les plus anciennes sont supprimées), réglable.

Le workload se déduit du pod : `ownerReferences` (ReplicaSet dont on retire `pod-template-hash` → Deployment ; StatefulSet et DaemonSet directement).

## 4. Configuration (`%APPDATA%\Ocekour\config.yaml`)

```yaml
theme: system # system | dark | light
bufferSize: 100000
updateShare: '\\srv\partage\ocekour' # optionnel

connections:
  - id: rancher-prod
    type: rancher
    name: Rancher prod
    url: https://rancher.intra
    tls: { insecure: false, caFile: 'C:\certs\ca-entreprise.cer' }
    clusters: # clusters cochés par l'utilisateur
      - { id: c-abc12, name: prod }
      - { id: c-def34, name: recette }
    # token : stocké chiffré à part (safeStorage)

profiles: # tous les profils qui correspondent sont fusionnés, le 1er prime ; `*` = défaut
  - match: 'prod/*/api-commandes' # cluster/namespace/workload
    fields: { time: '@timestamp', level: level, message: message }
    columns: [traceId, logger_name]
    expand: [stack_trace, payload]
    parseNested: true
  - match: '*'
    parseNested: false
```

Le fichier est validé par un schéma **zod**. En cas d'erreur, le message indique la ligne fautive et la dernière configuration valide reste active.

## 5. Librairies retenues

| Besoin                        | Librairie                                      |
| ----------------------------- | ---------------------------------------------- |
| Kubernetes / Rancher          | `@kubernetes/client-node`                      |
| SSH (v2)                      | `ssh2`                                         |
| YAML (commentaires conservés) | `yaml`                                         |
| Validation                    | `zod`                                          |
| Liste virtualisée             | `@angular/cdk/scrolling`                       |
| Éditeur YAML                  | CodeMirror 6 (`@codemirror/lang-yaml`)         |
| Packaging                     | `electron-builder` (cible `portable`)          |
| Build main/preload            | `esbuild`                                      |
| Tests                         | `vitest` (parser, config, matching de profils) |

## 6. Partage « copier / coller »

Le bouton « Copier le lien » produit un texte du type `ocekour://open?d=<base64 JSON>`. Il contient l'URL Rancher, l'id du cluster, le namespace, le pod ou workload, le conteneur, les options et les filtres. **Il ne contient jamais le token.**

Le collègue fait Ctrl+V dans Ocekour. L'application retrouve **sa propre** connexion ayant la même URL Rancher, ou lui propose d'en créer une. Si le pod n'existe plus, elle propose un pod du même workload.

## 7. Découpage

Découpage détaillé en tickets : [backlog.md](backlog.md).

- **v1** :
  - squelette Electron + Angular + Tailwind + thème + `.exe` portable ✅ ;
  - connexions Rancher (URL + token + TLS, sélection des clusters) ;
  - navigation cluster › namespace › pod › conteneur ;
  - suivi / historique / previous ;
  - parser + liste + détail + profils YAML + actions en un clic ;
  - filtres simples, onglets, export NDJSON, copie, lien de partage ;
  - mise à jour via le partage.
- **v2** : SSH, puis fichiers locaux ; multi-pods fusionnés d'un workload ; filtres `champ=valeur`.

## 8. Risques et points ouverts

1. **SmartScreen** : l'exe n'est pas signé, donc au premier lancement il faut cliquer « Informations complémentaires → Exécuter quand même ». Si une GPO bloque les exe non signés, il faudra une signature d'entreprise.
2. **Token Rancher à durée de vie limitée** : une erreur 401 affiche un message clair qui invite à le renouveler.
3. **Développer dans l'air-gap** (optionnel) : `npm install electron` télécharge un zip depuis GitHub. Pour ça il faut `ELECTRON_MIRROR` vers un dépôt Nexus _raw_, ou déposer le zip dans `%LOCALAPPDATA%\electron\Cache`.

## 9. Nouvelles sources : Kafka et S3 (validé)

Ordre de réalisation : **socle multi-sources → Kafka → S3**, puis la fin de la v1 (export/partage L11, mise à jour L12).

### 9.1 Socle multi-sources

- Une connexion a un **type** : `rancher`, `kafka` ou `s3` (union discriminée dans `config.yaml`, secrets toujours chiffrés à part).
- La fenêtre « Nouvelle connexion » commence par le choix du type ; chaque type a son formulaire et son bouton « Tester ».
- Un onglet a un **type de vue** : **flux** (conteneur Rancher, topic Kafka : liste, détail, filtres, profils…) ou **explorateur** (bucket S3).
- Profils, favoris et lien de partage fonctionnent pour tous les types. Clés de profil : `cluster/namespace/workload` (Rancher), `kafka/<connexion>/<topic>` (Kafka).

### 9.2 Kafka

| Sujet           | Décision                                                                                                                                                                                                                                                                                                                           |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bibliothèque    | **`@platformatic/kafka`** : 100 % JavaScript, maintenue, SASL SCRAM, client Schema Registry intégré (pas de module natif dans l'exe)                                                                                                                                                                                               |
| Connexion       | `bootstrap.servers` ; sécurité **`SASL_PLAINTEXT` + `SCRAM-SHA-512` par défaut**, avec possibilité de choisir `SASL_SSL` (même choix de certificat que Rancher), `PLAINTEXT`, SCRAM-SHA-256 ou PLAIN ; utilisateur + mot de passe chiffré. Une connexion par cluster, plusieurs possibles                                          |
| Topics          | Listés (≈ 100) avec filtre ; topics internes (`__…`) masqués ; regroupés par préfixe dans l'arbre ; favoris                                                                                                                                                                                                                        |
| Lecture         | **Sans groupe de consommateurs ni commit** : toutes les partitions sont lues directement, aucun impact sur les applications. Départ : N derniers messages (1 000 par défaut, répartis sur les partitions puis triés par date), « depuis » (5 min, 1 h, date — par horodatage des messages), ou « du début » ; puis suivi en direct |
| Affichage       | Une ligne par message : heure · partition · offset · clé · valeur. La valeur passe dans le même moteur que les logs (JSON, détail, filtres, profils en un clic). Le détail montre aussi la clé, les headers et les métadonnées ; la recherche porte aussi sur la clé et les headers                                                |
| Formats         | Détection automatique pour la valeur **et** la clé : **Avro** (octet magique `0` + id de schéma → **Schema Registry**), JSON, texte ; sinon hexadécimal                                                                                                                                                                            |
| Schema Registry | URL par connexion, **sans authentification**, HTTP ou HTTPS (certificats de confiance ou fichier `.cer`). Schémas mis en cache. Registre injoignable : message affiché en hexadécimal avec un avertissement                                                                                                                        |
| Écriture        | Aucune (lecture seule)                                                                                                                                                                                                                                                                                                             |

### 9.3 S3

| Sujet          | Décision                                                                                                                                                                                                                                                                                     |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bibliothèque   | **`@aws-sdk/client-s3`** (+ `@aws-sdk/lib-storage` pour les envois en plusieurs parties)                                                                                                                                                                                                     |
| Connexion      | S3 d'entreprise, **jamais AWS** : URL principale, région (facultative), mode « path-style », clé d'accès + clé secrète chiffrée, certificat comme pour Rancher                                                                                                                               |
| Buckets        | Listés (saisie manuelle possible) ; favoris                                                                                                                                                                                                                                                  |
| Vue            | **Explorateur** (pas une vue de logs) : dossiers / fichiers, taille, date, fil d'Ariane, tri, recherche par préfixe, chargement par pages pour les très gros dossiers                                                                                                                        |
| Aperçu         | Texte, JSON, XML, YAML, CSV, log… avec coloration (CodeMirror en lecture seule) ; `.gz` décompressé ; images ; gros fichiers : début seulement (5 Mo)                                                                                                                                        |
| Actions        | **Télécharger** (fichier, dossier), **téléverser** (bouton et glisser-déposer, fichiers et dossiers, progression), **renommer** (fichier et dossier : copie puis suppression), **supprimer** (fichier et dossier, avec confirmation indiquant le nombre d'objets), copier le chemin `s3://…` |
| Hors périmètre | Versions d'objets, « ouvrir comme logs »                                                                                                                                                                                                                                                     |

### 9.4 Tests

- **Kafka** : vrai broker Apache Kafka (mode KRaft, SCRAM-SHA-512) lancé localement pour les tests d'intégration ; Schema Registry simulé en Node (`/schemas/ids/<id>`). En CI : image Docker `apache/kafka`.
- **S3** : S3 simulé (`moto`) en local et en CI (l'image MinIO n'est plus publiée sur Docker Hub).
