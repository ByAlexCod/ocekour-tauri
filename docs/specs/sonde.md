# Spec — Sonde : exposer des logs d'une machine distante (2.2.0)

> Statut : **proposition** · S'appuie sur la lecture de fichiers de la
> [spec fichiers locaux](fichiers-locaux.md) (`streams/file.rs`), le mode sans interface
> (`ocekour-headless`) et le serveur HTTP du MCP (jeton, `hyper`).

## 1. Objectif

Beaucoup de logs ne sont ni dans Rancher ni dans Kafka : VM, serveurs Windows/Linux, machines
de recette, services installés à l'ancienne qui écrivent dans des fichiers. Aujourd'hui il faut
s'y connecter (RDP, SSH) et lire les fichiers à la main.

La **sonde** est le même exécutable, lancé **sur la machine distante** en mode `sonde` : il
**expose en lecture seule une ou plusieurs sources de logs** (fichiers, motifs de fichiers)
déclarées par l'administrateur. Côté poste, l'utilisateur ajoute une **connexion « Sonde »** :
les sources apparaissent dans l'arbre et s'ouvrent comme un pod (onglet, suivi, filtres,
assemblage, MCP).

Principes :

- **Même binaire, aucune interface** : `ocekour-headless sonde`, **Windows et Linux dès la
  2.2**, sans WebView2, quelques Mo de mémoire.
- **Indépendante de la clé d'activation** (§9).
- **Un jeton par sonde**, défini sur la sonde (§6).
- **C'est le poste qui initie** : le poste du développeur (l'interface) ouvre la connexion vers
  la sonde ; la sonde **écoute et répond, elle n'appelle jamais personne** (ni le poste, ni
  Internet). Seul son port est à ouvrir, en entrée, sur la machine distante.
- **Plusieurs personnes à la fois** : une sonde sert **plusieurs postes abonnés** aux mêmes
  sources ou à des sources différentes, chacun avec ses propres options, sans se gêner (§5).
- **Lecture seule, liste blanche** : seul ce qui est déclaré dans la configuration de la sonde
  est lisible ; aucun chemin arbitraire ne peut être demandé.
- **Sécurisé par défaut** : TLS obligatoire hors `127.0.0.1`, jeton d'accès, certificat épinglé
  côté poste (§6).
- **Réutilise le protocole des flux** : la sonde envoie les mêmes événements que le cœur
  (`lines`, `status`, `notice`) ; côté poste, une sonde est une connexion de plus.

Hors périmètre 2.2 : écrire ou supprimer des fichiers ; sonde qui se connecte vers le poste
(les postes joignent directement les machines : seul le port de la sonde est à ouvrir) ;
relais entre sondes. Les **autres types de sources** (journald, Observateur d'événements
Windows, `docker logs`, sortie de commande) viennent en **2.3** (§12).

## 2. Parcours

**Administrateur de la machine distante**

```
ocekour-headless sonde init                  # crée sonde.yaml, le certificat et le jeton
# édite sonde.yaml : déclare les sources
ocekour-headless sonde                       # démarre (premier plan), affiche l'adresse et l'empreinte
ocekour-headless sonde service installer     # service Windows ou unité systemd (§10)
```

`sonde init` affiche ce qu'il faut transmettre à l'utilisateur :

```
Sonde prête sur https://srv-recette-01:47120
Empreinte du certificat : SHA256:9F:2C:…:A1
Jeton : osk_3bb1e0c4…   (à transmettre de façon sûre ; affiché une seule fois,
                        `sonde jeton renouveler` en crée un nouveau et invalide l'ancien)
```

**Utilisateur (poste)** : Ajouter une connexion → **Sonde** → URL, jeton → « Tester ». Au
premier contact, Ocekour affiche l'empreinte du certificat à comparer avec celle fournie
(« Faire confiance à ce certificat »), puis l'épingle. Les sources apparaissent dans l'arbre :

```
SONDE  srv-recette-01
  ├─ api-commandes          (fichier)
  ├─ batch-nuit             (motif : 3 fichiers)
  │    ├─ batch-2026-10-22.log
  │    └─ …
  └─ iis-default            (motif)
```

## 3. Configuration de la sonde (`sonde.yaml`)

Fichier à côté de l'exe ou `--config=<chemin>` ; relu à chaud (sources ajoutées / retirées sans
redémarrage, flux en cours sur une source retirée → `ended`).

```yaml
ecoute: 0.0.0.0:47120          # adresse:port ; 127.0.0.1 = local seulement (TLS facultatif)
nom: srv-recette-01            # nom affiché côté poste (défaut : nom de la machine)
tls:
  certificat: sonde.crt        # créé par `sonde init` (autosigné, 10 ans) ou fourni (PKI)
  cle: sonde.key
jeton: "sha256:…"              # empreinte du jeton unique de la sonde, jamais le jeton en clair
sources:
  - id: api-commandes          # [a-z0-9-], stable : clé des liens, profils, favoris
    nom: API commandes
    fichier: /var/log/commandes/api.log
    encodage: utf-8            # utf-8 (défaut) | utf-16 | latin-1 | windows-1252
  - id: batch-nuit
    motif: /var/log/batch/batch-*.log   # liste les fichiers ; « dernier » = le plus récent
    garder: 30                 # au plus N fichiers listés (les plus récents)
limites:
  flux_simultanes: 50          # au total, tous postes confondus
  flux_par_poste: 10           # par poste abonné (adresse IP + identifiant de poste, §5)
  debit_max: 5 Mo/s            # par flux, au-delà : lignes sautées signalées (§5)
journal: sonde.log             # accès (qui, quelle source, quand) — jamais le contenu
```

Une **source** = `fichier` (un chemin) ou `motif` (glob dans **un seul dossier**, sans `**`).
Le modèle est extensible (`type: journald`, `type: evenements-windows`…) sans changer le
protocole : seuls `fichier` et `motif` sont dans la 2.2, les autres en 2.3 (§12).
Un motif se comporte comme en local (spec fichiers locaux §4 bis), avec le même code.

## 4. Protocole (HTTP/1.1, JSON)

Préfixe `/v1`. Toutes les requêtes portent `Authorization: Bearer <jeton>`.

| Requête                                       | Réponse                                                                                                    |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `GET /v1/info`                                | `{ nom, version, protocole: 1, os, sources: <nombre> }` — sert au « Tester »                                |
| `GET /v1/sources`                             | `[{ id, nom, type: "fichier"\|"motif", taille?, modifie?, etat: "ok"\|"absent"\|"illisible" }]`             |
| `GET /v1/sources/{id}/fichiers`               | motif : `[{ nom, taille, modifie }]`, du plus récent au plus ancien                                        |
| `GET /v1/sources/{id}/flux?…`                 | **flux NDJSON** (`Content-Type: application/x-ndjson`, chunked), un événement par ligne                     |

Paramètres du flux : `fichier=<nom>` (motif ; défaut : le plus récent, et **bascule
automatique** sur le nouveau fichier quand il apparaît), `lignes=N`, `depuis=<secondes>`,
`suivi=1`, `reprise=<curseur>`.

Événements (mêmes formes que `StreamEvent`, plus un curseur) :

```json
{"type":"lines","lines":[{"ts":"2026-10-22T06:46:04.262Z","text":"…"}],"curseur":"f:8812:1048576"}
{"type":"status","state":"live"}
{"type":"notice","message":"Fichier renouvelé (rotation)"}
{"type":"ping"}
```

- **Curseur** = identité du fichier + position : après une coupure réseau, le poste se
  reconnecte avec `reprise=<curseur>` → **ni perte ni doublon** (si le fichier a tourné entre
  temps : notice « rotation pendant la coupure », lecture du nouveau depuis le début).
- **`ping`** toutes les 15 s : détecte une connexion morte (proxy, pare-feu) ; le poste
  reconnecte au-delà de 45 s sans rien recevoir (même logique que Rancher : `reconnecting`).
- Pourquoi NDJSON plutôt que WebSocket / SSE / gRPC : traverse proxys et pare-feu HTTP,
  testable avec `curl`, aucune dépendance supplémentaire, même serveur `hyper` que le MCP.
- Erreurs : `401` jeton absent/invalide, `404` source ou fichier inconnu, `429` limite de flux
  atteinte, `503` source illisible (droits).

## 5. Plusieurs abonnés, charge et débit

Une sonde est partagée : plusieurs développeurs, chacun depuis son poste, s'**abonnent** à ses
sources (un abonnement = un flux ouvert par un onglet, un assemblage ou un outil MCP).

- **Indépendance** : chaque abonnement a ses options (`lignes`, `depuis`, `suivi`, fichier
  choisi dans un motif) et **sa propre position** ; l'un peut lire l'historique pendant qu'un
  autre suit en direct le même fichier.
- **Lecture partagée** : un fichier suivi par plusieurs abonnés n'est lu **qu'une fois** ;
  les nouvelles lignes sont diffusées à chacun (`broadcast.rs`). L'historique (N dernières
  lignes) est lu séparément pour chaque abonnement.
- **Pas de blocage mutuel** : chaque abonné a sa file bornée ; un poste lent (réseau, machine
  chargée) **ne ralentit pas les autres**. Si sa file déborde, il reçoit la notice
  « N lignes sautées (connexion trop lente) » et reprend au fil du direct.
- **Arrivées et départs** : un poste qui se connecte, se déconnecte ou perd le réseau n'a
  aucun effet sur les autres ; la reprise par curseur (§4) est propre à chaque abonnement.
- **Identification du poste** : le jeton étant commun, chaque requête porte l'en-tête
  informatif `X-Ocekour-Poste: <machine>/<utilisateur Windows>` (envoyé par l'interface). Il
  sert au journal, aux limites par poste et à `sonde statut` ; ce **n'est pas** une
  authentification.
- **`ocekour-headless sonde statut`** (sur la machine de la sonde) : abonnés en cours (poste,
  IP, source, depuis quand, débit, lignes sautées). Pas d'équivalent côté poste : un
  développeur ne voit pas qui d'autre lit la sonde.
- **Limites** : `flux_simultanes` (total) et `flux_par_poste` ; au-delà, `429` avec un message
  clair côté interface (« sonde saturée » / « trop d'onglets ouverts sur cette sonde »).
- Mémoire de la sonde bornée : < 30 Mo au repos, < 2 Mo par abonnement.
- Compression `Content-Encoding: gzip` négociée (logs texte : ÷5 à ÷10 sur le réseau).

## 6. Sécurité

| Risque                               | Mesure                                                                                                                                                       |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Lecture de fichiers non prévus       | Liste blanche (`sources`) ; le client ne fournit **jamais** de chemin, seulement un id de source et, pour un motif, un **nom** de fichier vérifié contre le glob (pas de `/`, `\`, `..`). Chemins canonisés, liens symboliques résolus puis revérifiés. |
| Écoute en clair sur le réseau        | TLS obligatoire si `ecoute` n'est pas une adresse de bouclage ; refus de démarrer sinon.                                                                      |
| Usurpation de la sonde               | Certificat autosigné **épinglé** par empreinte au premier contact (comparée par l'utilisateur), ou certificat d'une autorité d'entreprise (TLS habituel).       |
| Vol de jeton                         | **Un jeton par sonde**, défini sur la sonde : aléatoire 256 bits, stocké **haché** dans `sonde.yaml`, chiffré dans `secrets.json` sur le poste. Fuite → `sonde jeton renouveler` (l'ancien est refusé aussitôt, flux en cours fermés). Qui veut des accès séparés installe des sondes séparées (ports différents). |
| Brute force                          | Comparaison en temps constant ; 10 échecs/min par IP → blocage 5 min ; tout est journalisé.                                                                   |
| Déni de service                      | `flux_simultanes`, débit, taille des requêtes, délais de lecture d'en-têtes.                                                                                  |
| Données sensibles dans les logs      | Hors du rôle de la sonde : c'est l'administrateur qui choisit les sources exposées.                                                                          |
| Droits                               | Lancée avec un compte de service **non administrateur** qui n'a que la lecture des dossiers de logs (documenté pour Windows et Linux).                        |

## 7. Côté poste

- Nouveau type de connexion dans `config.yaml` :

  ```yaml
  - id: sonde-ab12cd34
    type: sonde
    name: srv-recette-01
    url: https://srv-recette-01:47120
    tls:
      empreinte: "SHA256:9F:2C:…:A1"   # épinglée ; ou insecure / caFile comme les autres
  ```

  Le jeton est le secret de la connexion (`secrets.json`, chiffré).
- **Arbre** : connexion › sources › (motif) fichiers. Santé : point vert si la source est
  lisible, gris si absente, rouge si illisible ; taille et âge du dernier changement.
- **Onglet** : `FileTarget { connectionId, path: [sourceId, fichier?] }` (spec fichiers
  locaux §3) ; toutes les fonctions d'un onglet de logs. Sur une source « motif » ouverte sans
  fichier précis, le suivi passe d'un fichier au suivant à la rotation (notice dans le flux).
- **Lien de partage** : `t: "sonde"`, `url` de la sonde, `source`, `fichier?` ; chez le
  destinataire, la connexion est retrouvée par URL, ou proposée à la création (sans jeton).
- **Assemblage** : sources de sondes, pods et topics mélangés.
- **Agents (MCP)** : `connexions_lister` montre les sondes ; `sources_parcourir` liste leurs
  sources ; `logs_lire` / `logs_exporter` acceptent le chemin `[source, fichier?]`. Pas de liste
  blanche supplémentaire : l'administrateur de la sonde a déjà choisi ce qui est exposé.

## 8. Implémentation

- `crates/core/src/sonde/` : `config.rs` (sonde.yaml, rechargement), `server.rs` (hyper +
  rustls, routes §4, auth, limites), `broadcast.rs` (un lecteur par fichier, N abonnés),
  `client.rs` (côté poste : `browse`, `spawn` d'un flux avec reconnexion et curseur).
- La lecture réutilise `streams/file.rs` (spec fichiers locaux) : même code, mêmes tests.
- `StreamManager::open` : `Connection::Sonde` → `sonde::client::spawn`.
- `ocekour-headless` : sous-commande `sonde [init | jeton renouveler | service
  installer|desinstaller | verifier]`. `verifier` relit la config et teste chaque source.
- Livrables 2.2 : **Windows** (`ocekour-headless.exe`, déjà construit) et **Linux**
  (`x86_64-unknown-linux-musl`, binaire statique sans dépendance système), tous deux en CI et
  testés sur leur système ; fonctionnalité Cargo `sonde-seule` pour un binaire réduit (sans
  Kafka, S3, navigateur, bases) si la taille compte. _(reco)_

## 9. Licence et statistiques

La sonde n'a **aucun rapport avec la clé d'activation** : elle n'en demande pas, n'en lit pas
et n'en transporte pas ; aucune préconfiguration de sonde par clé. Elle **n'envoie aucune
statistique** (serveur souvent sans accès Internet). Côté poste, l'ouverture d'un flux de sonde
est comptée comme les autres flux (`Flux / Ouvert / sonde`), sans nom de machine.

## 10. Exploitation

- **Service** : Windows (`sonde service installer` → service « Ocekour Sonde », démarrage
  automatique, compte `NT SERVICE\OcekourSonde`) ; Linux (unité systemd générée avec
  `DynamicUser=` ou utilisateur dédié, `ReadOnlyPaths=`).
- **Journal** : `sonde.log` (rotation 10 Mo × 5) : démarrage, accès (jeton, IP, source),
  refus, erreurs de lecture. Jamais le contenu des logs ni les jetons.
- **Mise à jour** : remplacer l'exe et redémarrer le service ; le protocole est versionné
  (`/v1`, `protocole` dans `/v1/info`) et le poste signale une sonde trop ancienne.
- Guide utilisateur `docs/guide/sonde.md` : installation pas à pas Windows et Linux, ouverture
  du port, transmission du jeton et de l'empreinte, dépannage.

## 11. Tests

- **Sonde** : init (fichiers créés, droits), config invalide refusée avec ligne fautive,
  rechargement à chaud, TLS obligatoire hors bouclage, jeton valide / invalide / renouvelé
  (ancien refusé, flux fermés), traversée (`..`, séparateurs, lien symbolique hors dossier)
  refusée, limites (429), blocage après échecs, démarrage sans aucune clé d'activation.
- **Flux de bout en bout** (vraie sonde sur un port libre + client du cœur) : N dernières
  lignes, suivi, rotation, bascule de motif, coupure réseau simulée puis **reprise sans perte
  ni doublon** par curseur, `ping` et détection de connexion morte.
- **Plusieurs abonnés** : 5 postes simulés sur une même source avec des options différentes
  (historique / suivi / autre fichier du motif), lecture unique du fichier, un abonné lent
  n'en retarde aucun autre (lignes sautées pour lui seul), départ d'un abonné sans effet sur
  les autres, limites par poste et totale (429), `sonde statut` exact, renouvellement du jeton
  qui ferme tous les abonnements.
- **Poste** : épinglage (empreinte changée → refus avec message clair), arbre, onglet,
  assemblage, lien de partage, outils MCP sur une source de sonde.
- **Binaire** : `ocekour-headless sonde` démarré par les tests (comme `--mcp-http`), sous
  Linux en CI et Windows dans le job de construction.

## 12. Décisions et suite

Décisions :

- **Réseau** : c'est **le poste qui initie** la connexion vers la sonde ; la sonde n'appelle
  jamais (pas de relais ni d'inversion).
- **Abonnés** : plusieurs postes abonnés en même temps, indépendants (§5).
- **Systèmes** : Windows **et** Linux dès la 2.2.
- **Jeton** : un seul par sonde, défini sur la sonde (§6).
- **Licence** : aucun lien avec la clé d'activation (§9).

Version suivante (2.3) : autres types de sources, chacun avec ses tests :

| Type                 | Système | Contenu                                                            |
| -------------------- | ------- | ------------------------------------------------------------------ |
| `journald`           | Linux   | unité(s) systemd, priorité → niveau, curseur journald pour la reprise |
| `evenements-windows` | Windows | journal (Application, System…) et source, niveau, `RecordId` pour la reprise |
| `docker`             | les deux | conteneur par nom ou label, `docker logs --follow` via l'API Docker |
| `commande`           | les deux | sortie d'une commande **déclarée** dans `sonde.yaml` (jamais fournie par le poste) |

Question ouverte : le **partage par lien** d'une source de sonde est-il utile tel quel (le
destinataire doit avoir le jeton), ou faut-il le désactiver ?
