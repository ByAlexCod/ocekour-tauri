# Ocekour — Découpage du développement

Référence fonctionnelle et technique : [cadrage.md](cadrage.md).

Chaque ticket porte une **taille** (S ≤ ½ j · M ≈ 1–2 j · L ≈ 3–5 j) et ses **dépendances**. Un ticket est _fini_ quand :

- le code est mergé, typé et formaté (prettier) ;
- les tests unitaires utiles sont écrits ;
- la CI est verte ;
- le comportement a été vérifié dans l'exe ;
- la doc est mise à jour si l'usage change.

---

## Vue d'ensemble et ordre de réalisation

| #    | Lot                                                | Jalon                            |
| ---- | -------------------------------------------------- | -------------------------------- |
| L0   | Socle et outillage                                 | ✅ en grande partie (PR #1)      |
| L1   | Configuration et secrets                           | **M1 – Connexion** — ✅          |
| L2   | Connexions Rancher                                 | M1 — ✅                          |
| L3   | Navigation cluster › namespace › pod › conteneur   | M1 — ✅                          |
| L4   | Streaming des logs (main → front)                  | **M2 – Lire des logs** — ✅      |
| L5   | Parsing et modèle `LogEvent`                       | M2 — ✅                          |
| L6   | Vue liste virtualisée                              | M2                               |
| L7   | Panneau détail (JSON lisible)                      | **M3 – Lecture intelligente**    |
| L8   | Profils par workload et configuration en un clic   | M3 — ✅                          |
| L9   | Filtres et recherche                               | M3 — ✅                          |
| L10  | Onglets                                            | **M4 – Confort et partage** — ✅ |
| L11  | Export, copie, lien de partage                     | M4                               |
| L12  | Mise à jour via partage réseau et release          | **M5 – Livraison v1**            |
| L13  | Robustesse, erreurs, journal applicatif            | transverse (M1 → M5)             |
| L14  | Qualité : tests, CI, documentation                 | transverse (M1 → M5)             |
| L15  | Socle multi-sources (types de connexion et de vue) | **M6 – Nouvelles sources** — ✅  |
| L16  | Source Kafka (+ Avro / Schema Registry)            | M6 — ✅                          |
| L17  | Source S3 (explorateur)                            | M6 — ✅                          |
| L20+ | v2 : SSH, fichiers locaux, multi-pods…             | après                            |

Ordre validé : **L15 → L16 → L17**, puis L11 et L12.

Chemin critique : **L1 → L2 → L3 → L4 → L5 → L6 → L7 → L8**. L9, L10 et L11 peuvent se faire en parallèle à partir de L6.

---

## L0 — Socle et outillage

| ID      | Ticket                                                                                                                                                                 | Taille | Dép.    | Statut                   |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------- | ------------------------ |
| L0-1    | Squelette Electron + Angular 22 + Tailwind, preload isolé, IPC typé (`shared/api.ts`)                                                                                  | M      | —       | ✅                       |
| L0-2    | Thème Système / Clair / Sombre persisté, aligné sur `nativeTheme`                                                                                                      | S      | L0-1    | ✅                       |
| L0-3    | Exe portable (`electron-builder --win portable`), icône                                                                                                                | S      | L0-1    | ✅                       |
| L0-4    | CI GitHub Actions : typecheck + tests + exe en artefact                                                                                                                | S      | L0-3    | ✅ à vérifier au 1er run |
| L0-5    | Conventions : ESLint (`angular-eslint`), `npm run lint`, prettier en CI                                                                                                | S      | L0-1    |                          |
| L0-6    | Mode dev : rechargement du process main à chaud (esbuild `--watch` + relance Electron)                                                                                 | S      | L0-1    |                          |
| 🟡 L0-7 | _(barre latérale redimensionnable par son bord, largeur mémorisée : faite)_ Mise en page applicative : barre latérale redimensionnable, zone centrale, barre de statut | S      | L0-1    |                          |
| L0-8    | Mémoriser taille, position et état maximisé de la fenêtre (`electron-window-state`)                                                                                    | S      | L0-1    |                          |
| L0-9    | Écran « À propos » : version, chemin du `config.yaml`, chemin du journal, bouton « ouvrir le dossier »                                                                 | S      | ✅ L1-1 |                          |

## L1 — Configuration et secrets

| ID      | Ticket                                                                                                                                       | Taille | Dép.    |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------- |
| ✅ L1-1 | Schéma zod complet de `config.yaml` (theme, bufferSize, updateShare, connections, profiles, defaults) + valeurs par défaut                   | M      | L0-1    |
| ✅ L1-2 | Chargement tolérant : YAML invalide → erreur visible (ligne/colonne) et dernière config valide conservée                                     | S      | ✅ L1-1 |
| ✅ L1-3 | Écriture ciblée qui conserve les commentaires (`YAML.Document`) : helpers `setIn` / `addToList` / `removeFromList`                           | S      | ✅ L1-1 |
| ✅ L1-4 | Surveiller les modifications externes du fichier (`fs.watch` + debounce) → rechargement à chaud et notification du front                     | S      | ✅ L1-2 |
| ✅ L1-5 | Coffre à secrets : `safeStorage` (DPAPI) → `secrets.bin` indexé par id de connexion ; API `get/set/delete` ; jamais exposé au front en clair | M      | L0-1    |
| ✅ L1-6 | Cas `safeStorage` indisponible : message explicite, pas de stockage en clair                                                                 | S      | ✅ L1-5 |
| ✅ L1-7 | Écran « Configuration » : éditeur YAML CodeMirror 6 (coloration, erreurs en ligne, Ctrl+S, bouton « Recharger »)                             | M      | ✅ L1-2 |
| ✅ L1-8 | Écran « Préférences » minimal : thème, taille du tampon, chemin du partage de mise à jour, nombre de lignes par défaut                       | S      | ✅ L1-1 |

## L2 — Connexions Rancher

| ID      | Ticket                                                                                                                                    | Taille | Dép.       |
| ------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------- |
| ✅ L2-1 | Formulaire « Nouvelle connexion » : nom, URL Rancher, token (saisie masquée), TLS (vérification normale / ignorer SSL / fichier `.cer`)   | M      | L1-3, L1-5 |
| ✅ L2-2 | Gestion du certificat : lecture `.cer/.crt/.pem`, conversion DER → PEM (`crypto.X509Certificate`), affichage sujet et date d'expiration   | S      | ✅ L2-1    |
| ✅ L2-3 | Client HTTP Rancher commun (undici/fetch + agent TLS de la connexion) avec timeouts et erreurs typées (401, 403, TLS, DNS, timeout)       | M      | ✅ L2-2    |
| ✅ L2-4 | Bouton « Tester la connexion » : `GET /v3` + identité (`/v3/users?me=true`) → OK / erreur lisible                                         | S      | ✅ L2-3    |
| ✅ L2-5 | Lister les clusters (`/v3/clusters`) : nom, id, état ; recherche ; cases à cocher ; sélection enregistrée dans `connections[].clusters`   | M      | ✅ L2-3    |
| ✅ L2-6 | Créer un `KubeConfig` en mémoire par cluster (serveur `…/k8s/clusters/<id>`, token, CA ou `skipTLSVerify`) pour `@kubernetes/client-node` | S      | ✅ L2-3    |
| ✅ L2-7 | Alternative : import d'un fichier kubeconfig Rancher (choix des contexts ; token déplacé dans le coffre)                                  | M      | ✅ L2-6    |
| ✅ L2-8 | Modifier, renommer, supprimer une connexion (suppression du secret associé), renouveler le token seul                                     | S      | ✅ L2-1    |
| ✅ L2-9 | Token expiré ou révoqué (401) : bandeau « Token expiré — le renouveler » qui ouvre directement le champ token                             | S      | ✅ L2-8    |

## L3 — Navigation

| ID      | Ticket                                                                                                                                                                                                                  | Taille | Dép.       |
| ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------- |
| ✅ L3-1 | Interface `LogSource` côté main (`browse`, `open`, `stop`) + registre des sources par type                                                                                                                              | S      | L0-1       |
| ✅ L3-2 | Arbre latéral : connexion › cluster › namespace › pod › conteneur, chargement paresseux, spinner et erreur par nœud                                                                                                     | L      | L2-6, L3-1 |
| ✅ L3-3 | Namespaces : `listNamespace` (les utilisateurs ont le droit de tout lister) ; un 403 éventuel s'affiche comme une erreur du nœud                                                                                        | S      | ✅ L3-2    |
| ✅ L3-4 | Pods : statut (Running / CrashLoop / Pending…), nombre de redémarrages, âge, workload déduit (`ownerReferences` ; ReplicaSet – `pod-template-hash` → Deployment)                                                        | M      | ✅ L3-2    |
| ✅ L3-5 | Conteneurs : conteneurs normaux **et** init containers, avec leur état ; « previous » proposé seulement s'il y a eu un redémarrage                                                                                      | S      | ✅ L3-4    |
| ✅ L3-6 | Filtre texte dans l'arbre (namespaces et pods), bouton « rafraîchir », mémorisation des nœuds dépliés                                                                                                                   | M      | ✅ L3-2    |
| ✅ L3-7 | Favoris : épingler un namespace ou un workload (★, stockés dans `config.yaml`) ; section **« ★ Favoris » toujours visible en haut** de la barre latérale, un workload favori se déplie directement sur ses pods actuels | S      | L3-2       |

## L4 — Streaming des logs

| ID      | Ticket                                                                                                                                                              | Taille | Dép.       |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------- |
| ✅ L4-1 | Ouverture d'un flux via `k8s.Log` : options `follow`, `tailLines`, `sinceSeconds`, `previous`, `timestamps: true`                                                   | M      | ✅ L3-5    |
| ✅ L4-2 | Découpage en lignes robuste (gestion des chunks coupés, `\r\n`, UTF-8 multi-octets)                                                                                 | S      | ✅ L4-1    |
| ✅ L4-3 | Envoi au front **par lots** (~100 ms ou 1 000 lignes) via `MessagePort` dédié par flux                                                                              | M      | ✅ L4-2    |
| ✅ L4-4 | Contrôle de flux : pause / reprise du suivi ; au-delà du tampon, lignes les plus anciennes supprimées ; compteur « lignes ignorées »                                | M      | ✅ L4-3    |
| ✅ L4-5 | Fin de flux ou coupure réseau : état visible (connecté / terminé / reconnexion…), reconnexion automatique avec `sinceTime` = dernier timestamp reçu (sans doublons) | M      | ✅ L4-1    |
| ✅ L4-6 | Pod redémarré ou supprimé pendant le suivi : message dans le flux et proposition de basculer sur le nouveau pod du même workload                                    | M      | L4-5, L3-4 |
| ✅ L4-7 | Barre d'options du flux : Suivi ⇄ Historique, N dernières lignes (100/**1 000 par défaut**/10 000/tout), « depuis » (5 min / 1 h / 24 h / date), previous           | S      | ✅ L4-1    |
| ✅ L4-8 | Fermeture propre : arrêt des requêtes HTTP quand l'onglet est fermé ou l'app quittée                                                                                | S      | ✅ L4-1    |

## L5 — Parsing et modèle

| ID      | Ticket                                                                                                                                                                           | Taille | Dép.       |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------- |
| ✅ L5-1 | Modèle `LogEvent` : `seq`, `raw`, `k8sTime`, `time`, `level`, `message`, `json?`, `source`                                                                                       | S      | —          |
| ✅ L5-2 | Parser : séparation du timestamp k8s ; `{…}` + `JSON.parse` OK → JSON, sinon texte ; lignes énormes (> 1 Mo) tronquées pour la vue, texte brut conservé                          | M      | ✅ L5-1    |
| ✅ L5-3 | Détection auto des champs heure / niveau / message (liste de candidats, chemins pointés `log.level`) + surcharge par profil                                                      | S      | ✅ L5-2    |
| ✅ L5-4 | Normalisation du niveau : texte (`WARN`, `warning`, `W`, `Error`…) et numérique (pino 10–60, syslog 0–7) → `trace/debug/info/warn/error/fatal/unknown`                           | S      | ✅ L5-3    |
| ✅ L5-5 | Dates : ISO, epoch s/ms/ns, formats Logback courants → `Date` ; affichage en **heure locale du poste** avec millisecondes                                                        | S      | ✅ L5-3    |
| ✅ L5-6 | Logs texte : niveau deviné par regex (`\b(ERROR                                                                                                                                  | WARN   | INFO       | DEBUG)\b`), codes couleur ANSI rendus (`anser`) ou retirés | S   | ✅ L5-2 |
| ✅ L5-7 | `parseNested` : une chaîne qui commence par `{` ou `[` et qui se parse → sous-objet (profondeur max 3, sans bloquer le rendu)                                                    | S      | ✅ L5-2    |
| ⏭️ L5-8 | _(inutile à ce stade : mesuré à ~1 000 lignes/s sans blocage notable)_ Parsing dans un **Web Worker** si le débit dépasse les capacités du thread UI (mesurer avant de le faire) | M      | L5-2, L6-1 |
| ✅ L5-9 | Tests unitaires du parser : jeux de lignes réalistes (Logback JSON, pino, Serilog, zap, texte, ANSI, lignes cassées)                                                             | M      | L5-2…L5-7  |

## L6 — Vue liste

| ID      | Ticket                                                                                                                                                                                        | Taille | Dép.       |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------- |
| ✅ L6-1 | Tampon circulaire par onglet (taille `bufferSize`) + signal Angular de version pour le rendu                                                                                                  | S      | ✅ L5-1    |
| ✅ L6-2 | Liste virtualisée `@angular/cdk/scrolling`, hauteur de ligne fixe, police monospace                                                                                                           | M      | ✅ L6-1    |
| 🟡 L6-3 | _(tout fait sauf le redimensionnement des colonnes)_ Colonnes : heure · niveau (badge coloré) · message sur une ligne (`\n` → `⏎`, tronqué) + colonnes du profil ; largeurs redimensionnables | M      | L6-2, L8-1 |
| ✅ L6-4 | Défilement auto « coller en bas » ; il se désactive si l'utilisateur remonte, avec un bouton « ↓ N nouvelles lignes »                                                                         | S      | ✅ L6-2    |
| 🟡 L6-5 | _(clic et ↑/↓/Page/Début/Fin faits ; sélection multiple avec L11)_ Sélection d'une ligne (clic, ↑/↓) → ouvre le détail ; sélection multiple (Maj/Ctrl) pour la copie                          | M      | ✅ L6-2    |
| ✅ L6-6 | Couleurs de ligne selon le niveau (fond léger pour error/warn), lisibles en sombre **et** en clair                                                                                            | S      | L6-3       |
| ✅ L6-7 | Barre de statut : lignes affichées / total, débit (lignes/s), état du flux, lignes supprimées                                                                                                 | S      | L6-1, L4-5 |
| ✅ L6-8 | Test de charge : 1 000 lignes/s pendant 5 min sans perte de fluidité ni fuite mémoire                                                                                                         | S      | ✅ L6-2    |

## L7 — Panneau détail

| ID      | Ticket                                                                                                                           | Taille | Dép.       |
| ------- | -------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------- |
| ✅ L7-1 | Panneau latéral redimensionnable (ou en bas, au choix), fermeture par Échap, navigation ↑/↓ sans le fermer                       | S      | L6-5       |
| ✅ L7-2 | Arbre JSON repliable (composant maison récursif, léger), clé/valeur typées et colorées                                           | M      | ✅ L7-1    |
| ✅ L7-3 | **Chaînes multi-lignes** rendues telles quelles (`white-space: pre-wrap`), indentation conservée, bloc à défilement si très long | S      | ✅ L7-2    |
| ✅ L7-4 | Champs `expand` du profil dépliés et affichés en tête, en grand                                                                  | S      | L7-2, L8-1 |
| ✅ L7-5 | Sous-objets issus de `parseNested` affichés en sous-arbre, avec un bouton « voir la chaîne brute »                               | S      | L7-2, L5-7 |
| ✅ L7-6 | Actions par champ : copier la valeur, copier le chemin, filtrer sur cette valeur (L9)                                            | S      | ✅ L7-2    |
| ✅ L7-7 | Actions globales : copier le JSON formaté, copier la ligne brute, onglet « Brut »                                                | S      | ✅ L7-1    |
| ✅ L7-8 | Logs texte : affichage multi-ligne du message complet, sans arbre                                                                | S      | ✅ L7-1    |
| ✅ L7-9 | Recherche dans le détail (Ctrl+F local)                                                                                          | S      | ✅ L7-2    |

## L8 — Profils et configuration en un clic

| ID      | Ticket                                                                                                                                                           | Taille | Dép.             |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------------- |
| ✅ L8-1 | Résolution du profil actif : clé `cluster/namespace/workload`, jokers `*` (`picomatch`), le premier qui correspond gagne, fusion avec les valeurs par défaut     | S      | L1-1, L3-4       |
| ✅ L8-2 | Affichage du profil actif dans l'onglet + lien « éditer » (ouvre le YAML positionné sur ce profil)                                                               | S      | L8-1, L1-7       |
| ✅ L8-3 | Action « Toujours déplier ce champ pour ce workload » depuis le détail → crée ou complète le profil exact et l'écrit dans le YAML en conservant les commentaires | M      | L8-1, L1-3, L7-6 |
| ✅ L8-4 | Action « Afficher en colonne » / « Retirer la colonne »                                                                                                          | S      | ✅ L8-3          |
| ✅ L8-5 | Action « Utiliser ce champ comme heure / niveau / message »                                                                                                      | S      | ✅ L8-3          |
| ✅ L8-6 | Bascule `parseNested` pour le workload depuis la barre d'outils                                                                                                  | S      | ✅ L8-3          |
| ✅ L8-7 | Application **à chaud** : tout changement de profil re-rend les lignes déjà affichées (re-parse du tampon)                                                       | S      | L8-1, L1-4       |
| ✅ L8-8 | Tests : correspondance des profils, écriture YAML ciblée avec commentaires préservés                                                                             | S      | L8-1, L8-3       |

## L9 — Filtres et recherche

| ID      | Ticket                                                                                                                                 | Taille | Dép.       |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------- |
| ✅ L9-1 | Champ de filtre texte (Ctrl+F) : sous-chaîne insensible à la casse par défaut, options Aa et `.*` (regex, erreur affichée si invalide) | M      | ✅ L6-2    |
| ✅ L9-2 | Filtre par niveaux (cases à cocher avec compteurs)                                                                                     | S      | L5-4, L6-2 |
| ✅ L9-3 | Mode « filtrer » (masque le reste) ⇄ mode « surligner » (tout visible, correspondances en surbrillance, F3 / Maj+F3 pour naviguer)     | M      | ✅ L9-1    |
| ✅ L9-4 | Surlignage des termes dans la liste **et** dans le détail                                                                              | S      | L9-1, L7-2 |
| ✅ L9-5 | Le filtre s'applique aussi aux lignes qui arrivent en direct ; performances correctes sur 100 000 lignes                               | S      | ✅ L9-1    |
| ✅ L9-6 | « Afficher le contexte » : depuis une ligne filtrée, retrouver sa position dans le flux complet                                        | S      | ✅ L9-3    |

## L10 — Onglets

| ID       | Ticket                                                                                                                       | Taille | Dép.        |
| -------- | ---------------------------------------------------------------------------------------------------------------------------- | ------ | ----------- |
| ✅ L10-1 | Un onglet = un flux (titre `namespace/pod/conteneur`, pastille d'état, indicateur de nouvelles erreurs)                      | M      | L4-3, L6-2  |
| ✅ L10-2 | Ouvrir depuis l'arbre (clic = onglet courant, clic milieu / Ctrl+clic = nouvel onglet), fermer (Ctrl+W), naviguer (Ctrl+Tab) | S      | ✅ L10-1    |
| ✅ L10-3 | Filtres, options et scroll conservés par onglet                                                                              | S      | ✅ L10-1    |
| ✅ L10-4 | Réordonner les onglets par glisser-déposer (`@angular/cdk/drag-drop`)                                                        | S      | ✅ L10-1    |
| ✅ L10-5 | Rouvrir les onglets de la session précédente au démarrage (activé par défaut, désactivable dans les préférences)             | S      | L10-1, L1-1 |

## L11 — Export, copie, partage

| ID       | Ticket                                                                                                                                                                                                                                                                             | Taille | Dép.        |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----------- |
| ✅ L11-1 | Export du tampon (tout ou filtré) : NDJSON (JSON original) ou texte brut, via la boîte « Enregistrer sous » native (lignes affichées, filtres compris ; format choisi par l'extension)                                                                                             | S      | ✅ L6-1     |
| L11-2    | Copie des lignes sélectionnées (brut ou JSON formaté) — _en attendant la multi-sélection (L6-5) : copie d'une ligne depuis le détail_                                                                                                                                              | S      | L6-5        |
| ✅ L11-3 | Format du lien de partage `ocekour://open?d=<base64url JSON>` versionné (`v:1`) : type de source + cible (Rancher : URL, cluster, namespace, pod, workload, conteneur ; Kafka : bootstrap, topic ; S3 : URL, bucket, dossier), options, filtres. **Jamais de secret**              | S      | ✅ L10-1    |
| ✅ L11-4 | « Copier le lien » dans l'onglet                                                                                                                                                                                                                                                   | S      | L11-3       |
| ✅ L11-5 | Coller un lien (Ctrl+V dans l'app ou bouton « Ouvrir un lien ») — seul mode d'ouverture, pas de protocole cliquable : validation zod, connexion retrouvée par URL Rancher ; sinon proposition d'en créer une (URL préremplie) ; cluster non sélectionné → proposition de l'ajouter | M      | L11-3, L2-1 |
| ✅ L11-6 | Pod introuvable → proposer les pods actuels du même workload                                                                                                                                                                                                                       | S      | L11-5, L3-4 |

## L12 — Mise à jour et release

| ID    | Ticket                                                                                                                                                        | Taille | Dép.    |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------- |
| L12-1 | Au démarrage (en arrière-plan, timeout 3 s) : lister `updateShare`, repérer `Ocekour-<semver>.exe` le plus récent (`semver`)                                  | S      | ✅ L1-1 |
| L12-2 | Bandeau « Version X disponible » → ouvre le dossier du partage dans l'Explorateur ; bouton « ignorer cette version »                                          | S      | L12-1   |
| L12-3 | Partage absent ou inaccessible : aucun message bloquant, trace dans le journal                                                                                | S      | L12-1   |
| L12-4 | Métadonnées de l'exe : version, nom produit, copyright (onglet Détails de Windows)                                                                            | S      | L0-3    |
| L12-5 | Processus de release : bump de version, tag `vX.Y.Z`, CI qui publie l'exe en GitHub Release, CHANGELOG                                                        | S      | L0-4    |
| L12-6 | Procédure écrite : dépose sur le partage, premier lancement (SmartScreen), emplacement de la config, désinstallation (supprimer l'exe et `%APPDATA%\Ocekour`) | S      | L12-5   |

## L13 — Robustesse, erreurs, journal (transverse)

| ID       | Ticket                                                                                                                                                                                                             | Taille | Dép.        |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ | ----------- |
| ✅ L13-1 | Journal applicatif `electron-log` → `%APPDATA%\Ocekour\logs\main.log` (rotation), erreurs du front relayées                                                                                                        | S      | L0-1        |
| L13-2    | Gestion globale des erreurs : `uncaughtException` / `unhandledRejection` côté main, `ErrorHandler` Angular → toast + journal                                                                                       | S      | ✅ L13-1    |
| ✅ L13-3 | Système de notifications (toasts) et d'états vides / chargement / erreur cohérents                                                                                                                                 | S      | L0-7        |
| L13-4    | Messages d'erreur réseau traduits en français et actionnables (TLS → « cocher ignorer SSL ou fournir le .cer », DNS, timeout, 403 « droits insuffisants sur … »)                                                   | S      | ✅ L2-3     |
| 🟡 L13-5 | Sécurité _(CSP et validation IPC de la config faites)_ : aucune donnée de log interprétée en HTML (pas de `innerHTML`), CSP stricte sur la page, validation zod de **tous** les messages IPC entrants dans le main | S      | L0-1        |
| L13-6    | Masquage des tokens dans le journal et dans les messages d'erreur                                                                                                                                                  | S      | ✅ L13-1    |
| L13-7    | Raccourcis clavier documentés (écran d'aide, `?`)                                                                                                                                                                  | S      | L10-2, L9-1 |

## L14 — Qualité, tests, documentation (transverse)

| ID       | Ticket                                                                                                                             | Taille | Dép.            |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------ | --------------- |
| L14-1    | Tests unitaires vitest côté main (config, secrets mockés, TLS/cert, construction du kubeconfig, lien de partage)                   | M      | au fil de l'eau |
| ✅ L14-2 | Faux serveur Rancher/K8s (Node `http`) pour les tests d'intégration : `/v3/clusters`, namespaces, pods, logs en streaming, 401/403 | M      | L2-5, L4-1      |
| L14-3    | Tests e2e Playwright `_electron` sous xvfb en CI : connexion → arbre → flux → filtre → détail → action de profil                   | M      | L14-2, M3       |
| L14-4    | Recette manuelle sur Windows 10 et 11 réels avec le vrai Rancher 2.5.5 (grille de recette dans `docs/recette.md`)                  | M      | M5              |
| L14-5    | Guide utilisateur (FR) : premiers pas, connexion, lecture, profils, partage, raccourcis                                            | S      | M4              |
| L14-6    | Référence de `config.yaml` générée depuis le schéma zod, avec exemples commentés                                                   | S      | ✅ L1-1         |

---

## L15 — Socle multi-sources

| ID       | Ticket                                                                                                                                        | Taille | Dép.     |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ------ | -------- |
| ✅ L15-1 | Schéma : union `rancher` / `kafka` / `s3` pour `connections`, secrets par type (token, mot de passe, clé secrète)                             | S      | —        |
| ✅ L15-2 | Fenêtre de connexion : choix du type, formulaire propre à chaque type, « Tester » commun ; TLS (confiance Windows, `.cer`, ignorer) factorisé | M      | ✅ L15-1 |
| ✅ L15-3 | Registre des sources côté main : `browse` / `open` selon le type ; arbre avec icône par type                                                  | M      | ✅ L15-1 |
| ✅ L15-4 | Onglets typés : vue **flux** (existante) ou vue **explorateur** ; restauration au démarrage pour les deux                                     | M      | L10      |
| ✅ L15-5 | Profils : clé par type (`cluster/namespace/workload`, `kafka/<connexion>/<topic>`) ; favoris par type                                         | S      | L8       |

## L16 — Source Kafka

| ID       | Ticket                                                                                                                                                                                                                                                                               | Taille | Dép.         |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ | ------------ |
| ✅ L16-1 | Connexion : bootstrap servers ; sécurité `SASL_PLAINTEXT`/`SCRAM-SHA-512` par défaut, `SASL_SSL`, `PLAINTEXT`, SCRAM-SHA-256, PLAIN ; utilisateur / mot de passe chiffré ; URL du Schema Registry (HTTP/HTTPS, confiance ou `.cer`) ; « Tester » (métadonnées du cluster + registre) | M      | ✅ L15-2     |
| ✅ L16-2 | Topics : liste (internes `__…` masqués), regroupement par préfixe, filtre, favoris, nombre de partitions                                                                                                                                                                             | M      | ✅ L15-3     |
| ✅ L16-3 | Lecture sans groupe ni commit, toutes partitions : N derniers messages (répartis par partition, fusionnés par date), « depuis » (offsets par horodatage), « du début », puis suivi en direct ; arrêt propre à la fermeture de l'onglet                                               | L      | L16-1        |
| ✅ L16-4 | Décodage clé et valeur : Avro (octet magique + id → registre, cache des schémas, `avsc`), JSON, texte, sinon hexadécimal ; registre injoignable → avertissement                                                                                                                      | M      | L16-3        |
| ✅ L16-5 | Vue flux : colonnes partition · offset · clé ; détail avec clé, headers, métadonnées ; recherche sur clé et headers ; profils par topic                                                                                                                                              | M      | L16-4, L15-5 |
| ✅ L16-6 | Erreurs actionnables : authentification refusée, broker injoignable, topic supprimé, droits insuffisants ; reconnexion automatique en suivi                                                                                                                                          | M      | L16-3        |
| ✅ L16-7 | Tests : broker Kafka réel (KRaft + SCRAM) en local et en CI (`scripts/kafka-test-broker.sh`, distribution Apache), registre simulé ; production de messages Avro/JSON/texte de test                                                                                                  | M      | L16-3        |

## L17 — Source S3

| ID       | Ticket                                                                                                                                                                 | Taille | Dép.         |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------------ |
| ✅ L17-1 | Connexion : URL, région facultative, path-style, clé d'accès / clé secrète chiffrée, TLS ; « Tester » (liste des buckets)                                              | S      | ✅ L15-2     |
| ✅ L17-2 | Buckets : liste, saisie manuelle, favoris ; ouverture d'un bucket = onglet explorateur                                                                                 | S      | L17-1, L15-4 |
| ✅ L17-3 | Explorateur : dossiers / fichiers (délimiteur `/`), taille, date, fil d'Ariane, tri, recherche par préfixe, pagination (continuation) avec liste virtualisée           | L      | L17-2        |
| ✅ L17-4 | Aperçu : texte, JSON, XML, YAML, CSV, log avec coloration (CodeMirror lecture seule), `.gz` décompressé, images ; début seulement (5 Mo) pour les gros fichiers        | M      | L17-3        |
| ✅ L17-5 | Télécharger un fichier (boîte « Enregistrer sous », en streaming) ou un dossier (choix du dossier local, récursif, progression)                                        | M      | L17-3        |
| ✅ L17-6 | Téléverser fichiers et dossiers : bouton + glisser-déposer, envoi en plusieurs parties, progression, annulation                                                        | M      | L17-3        |
| ✅ L17-7 | Renommer (fichier : copie + suppression ; dossier : récursif) et supprimer (fichier, dossier récursif) avec confirmation (nombre d'objets) ; copier le chemin `s3://…` | M      | L17-3        |
| ✅ L17-8 | Tests : S3 simulé (`moto`, `scripts/s3-test-server.sh`) en local et en CI ; traduction des erreurs testée à part                                                       | M      | L17-1        |

## L18 — Hub MCP (0.3.0) · spec [specs/hub-mcp.md](specs/hub-mcp.md)

| ID       | Ticket                                                                                                                                              | Taille | Dép.  |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----- |
| ✅ L18-1 | Comptes : `services` dans config.yaml (URL, TLS, activation, écriture), jeton chiffré, test, page de création de jeton, préconfiguration par la clé | M      | —     |
| ✅ L18-2 | Outils GitLab (lecture ; commentaires globaux ou positionnés, issues, MR, relances)                                                                 | L      | L18-1 |
| ✅ L18-3 | Outils Jira (JQL, tickets, projets, sprints ; création, modification, commentaires, transitions, assignation, temps) + Markdown → wiki              | M      | L18-1 |
| ✅ L18-4 | Outils Confluence (CQL, pages en Markdown, espaces, pièces jointes ; création, modification avec contrôle de version, commentaires)                 | M      | L18-1 |
| ✅ L18-5 | Outils Rocket.Chat (salons, historique, fils, recherche ; messages, réactions)                                                                      | S      | L18-1 |
| ✅ L18-6 | Signature des écritures, journal `actions.ndjson`, page « Actions de l'agent » avec pastille                                                        | M      | L18-2 |
| ✅ L18-7 | Aide Mistral Vibe (vérifiée avec le vrai client), documentation                                                                                     | S      | —     |
| L18-8    | _(plus tard)_ Agréger d'autres serveurs MCP derrière Ocekour ; outils combinés (logs ↔ pipeline ↔ ticket) ; intégration dans l'interface            | L      | L18-6 |

## L19 — Notifications vers Rocket.Chat (0.4.0) · spec [specs/notifications.md](specs/notifications.md)

| ID       | Ticket                                                                                                                           | Taille | Dép.  |
| -------- | -------------------------------------------------------------------------------------------------------------------------------- | ------ | ----- |
| ✅ L19-1 | Config `notifications` (activation, intervalle, destinataire, sources), état `notifications.json` (curseurs, anti-doublon, file) | S      | L18-1 |
| ✅ L19-2 | Lecteurs GitLab (To-Do), Jira (JQL + changelog + mentions), Confluence (CQL) ; message groupé                                    | M      | L19-1 |
| ✅ L19-3 | Scrutateur (boucle, erreurs, Matomo), envoi Rocket.Chat à soi-même, message de confirmation                                      | M      | L19-2 |
| ✅ L19-4 | Écran Paramètres → Notifications (test), mode `--notify`, docs                                                                   | S      | L19-3 |
| L19-5    | _(plus tard)_ Flux d'activité Jira/Confluence pour les mentions isolées ; canaux multiples ; résumé quotidien                    | M      | L19-3 |

## v2 (après livraison v1)

| ID   | Ticket                                                                                                                                                                                                                                                                                | Taille |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| L20  | **Source SSH** (`ssh2`) : hôte, port, utilisateur ; **auth par clé** (fichier + passphrase dans le coffre) ; empreinte de l'hôte confirmée au 1er accès puis mémorisée ; lecture par **`tail -n <N> -F <fichier>`** ; navigation dans les fichiers (`ls` sur des dossiers configurés) | L      |
| L20b | SSH _(si besoin)_ : rebond (jump host), fichiers gzip de rotation (`zcat`)                                                                                                                                                                                                            | M      |
| L21  | **Fichiers locaux** : ouvrir ou glisser un fichier, suivi (`chokidar`), gestion de la rotation, lecture de la fin des très gros fichiers sans tout charger                                                                                                                            | M      |
| L22  | **Multi-pods fusionnés** d'un workload (façon `stern`) : couleur par pod, tri par timestamp, pods qui arrivent ou partent                                                                                                                                                             | L      |
| L23  | **Filtres avancés** `champ=valeur`, `champ~regex`, `niveau>=warn`, combinaison ET/OU                                                                                                                                                                                                  | M      |
| L24  | Filtres enregistrés et marque-pages                                                                                                                                                                                                                                                   | S      |
| L25  | Coloration SQL / XML / stacktrace dans le détail (`highlight.js`, langues ciblées)                                                                                                                                                                                                    | S      |
| L26  | Plusieurs flux côte à côte (vue divisée)                                                                                                                                                                                                                                              | M      |
| L27  | Signature de l'exe (si une GPO l'impose)                                                                                                                                                                                                                                              | S      |

---

## Décisions complémentaires (validées)

1. Heure affichée : **heure locale du poste**.
2. **1 000 lignes** chargées par défaut à l'ouverture d'un flux.
3. **Restauration des onglets** de la session précédente au démarrage.
4. Partage **uniquement par copier/coller**, pas de protocole `ocekour://` cliquable.
5. Les utilisateurs Rancher peuvent **lister tous les namespaces**, donc pas de mécanisme de repli.
6. SSH (v2) : **authentification par clé**, lecture par **`tail -F`**.
