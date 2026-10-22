# Spec — Clé d'activation d'Ocekour

> Statut : **validée, implémentée en 0.2.0** · Guide administrateur : [docs/cles-activation.md](../cles-activation.md)

## 1. Objectif

Ocekour ne s'utilise qu'avec une **clé d'activation**. Cette clé :

- est **sans date d'expiration** ;
- porte la **configuration Matomo** (adresse, site…), qui n'est donc plus ni dans
  `config.yaml` ni dans l'exe ;
- fixe le **niveau de suivi**. L'objectif est d'**inciter** au reporting, pas de faire une
  protection anti-piratage (voir §7).

## 2. Les trois types de clé

| Type                               | Matomo dans la clé | Matomo injoignable                                                                                      | Usage visé                                 |
| ---------------------------------- | ------------------ | ------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| **Sans suivi** (`none`)            | non                | sans objet : rien n'est envoyé                                                                          | Postes hors réseau, cas particuliers       |
| **Suivi si possible** (`optional`) | oui                | l'application fonctionne ; les statistiques sont gardées en file puis abandonnées (comportement actuel) | Cas général                                |
| **Suivi obligatoire** (`required`) | oui                | l'application est **bloquée** tant que Matomo n'a pas répondu (voir §5)                                 | Équipes dont on veut garantir le reporting |

Il n'y a plus de case « désactiver les statistiques » dans les Paramètres : **c'est la clé qui
décide**.

## 3. Contenu d'une clé

```jsonc
{
  "v": 1, // version du format
  "id": "k-7f3a9c", // identifiant unique (révocation, statistiques)
  "label": "Équipe Paiement", // à qui la clé a été remise (affiché dans l'appli)
  "issuedAt": "2026-09-24", // date d'émission (information seulement)
  "mode": "optional", // none | optional | required
  "matomo": {
    // absent si mode = none
    "url": "https://matomo.intra/",
    "siteId": 12,
    "insecure": false, // ignorer la vérification TLS (déconseillé)
    "dimensions": { "version": 1, "key": 2 }, // dimensions personnalisées (facultatif)
  },
}
```

La dimension `key` envoie l'**id** de la clé (pas son libellé). Dans Matomo, on peut ainsi
segmenter l'usage par clé, donc par équipe, sans nom de personne.

## 4. Format, génération, vérification

### 4.1 Cryptographie

| Besoin                                                                                        | Mécanisme                                                                                                                     |
| --------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| **Personne ne fabrique ni ne modifie une clé** (passer en « sans suivi », changer de Matomo…) | Signature **Ed25519**. La clé privée reste chez l'administrateur ; seules les **clés publiques** sont dans l'exe              |
| **La configuration Matomo n'est pas lisible dans la clé**                                     | Contenu chiffré en **AES-256-GCM** avec une clé d'obfuscation injectée à la compilation (secret GitHub `LICENCE_OBFUSCATION`) |
| **Plusieurs clés de signature possibles** (rotation)                                          | Champ `kid`. L'exe accepte une liste de clés publiques                                                                        |

Format texte, à copier-coller :

```
OCEKOUR1-<kid>.<base64url(nonce ‖ contenu chiffré)>.<base64url(signature)>
```

Cela représente environ 350 caractères, sur une seule ligne. La signature porte sur tout ce
qui la précède.

### 4.2 Outil de génération (administrateur)

Un script Node dans le dépôt, `scripts/cle.mjs`, sans dépendance :

```bash
# Une fois : paire de clés de signature
node scripts/cle.mjs init --kid prod1
#  → ocekour-prod1.pem   : clé PRIVÉE, à garder hors du dépôt (coffre de mots de passe)
#  → affiche la clé PUBLIQUE, à ajouter dans electron/licence/cles-publiques.ts (commitée)

# Générer une clé pour une équipe
LICENCE_OBFUSCATION=… node scripts/cle.mjs generer --prive ocekour-prod1.pem \
  --label "Équipe Paiement" --mode optional \
  --matomo-url https://matomo.intra/ --site-id 12 --dim-version 1 --dim-cle 2
#  → OCEKOUR1-prod1.q8Fz….Xy3k

# Relire / vérifier une clé (support)
LICENCE_OBFUSCATION=… node scripts/cle.mjs lire OCEKOUR1-prod1.….…
```

Un fichier `cles-emises.csv` (id, libellé, type, date, destinataire) est tenu à jour par
l'administrateur, **hors dépôt**, pour la traçabilité.

### 4.3 Vérification dans l'application (process main)

1. Format et `kid` connus, puis **signature valide**. Sinon : « Clé invalide ou altérée ».
2. Déchiffrement et validation du contenu (zod). Sinon : « Clé illisible ».
3. `v` supporté. Sinon : « Clé créée pour une version plus récente d'Ocekour ».
4. `id` absent de la **liste de révocation** compilée dans l'exe. Sinon : « Clé révoquée ».

## 5. Comportement

### 5.1 Activation

- **Premier lancement sans clé** : écran plein « Activer Ocekour », avec un champ pour coller
  la clé et un bouton **Activer**. Le reste de l'application est inaccessible.
- **Clé acceptée** : un résumé s'affiche (« Clé de _Équipe Paiement_ · Suivi si
  possible »), puis l'application démarre.
- **Stockage** : la clé est enregistrée dans `secrets.json`, **chiffrée par la clé locale**
  (`secret.key`, depuis la 0.5.0 ; DPAPI avant), comme les tokens. Elle ne va **jamais** dans `config.yaml`, jamais dans le
  journal, et n'est jamais renvoyée à l'interface après activation.
- **Paramètres → Licence** : libellé, type, date d'émission, id, bouton **Changer de clé**.
  L'adresse Matomo n'est **pas affichée**.
- **Mise à jour depuis la 0.1.x** : l'écran d'activation apparaît une fois ; les connexions
  et la configuration sont conservées.

### 5.2 « Suivi obligatoire »

- **Au démarrage** : Ocekour envoie l'événement de démarrage à Matomo et **attend une réponse
  2xx** (10 s au plus). En cas d'échec, un écran bloquant s'affiche : « Ocekour doit joindre
  le serveur de statistiques de l'équipe (réseau, VPN ?) », avec les boutons **Réessayer**
  et **Changer de clé**.
- **En cours d'utilisation** : les envois ont lieu toutes les 30 s et un signal toutes les
  5 min. Si **aucun envoi n'a réussi depuis 15 min**, l'écran bloquant s'affiche par-dessus
  l'application. Les onglets et les flux sont conservés en arrière-plan, et tout reprend dès
  qu'un envoi réussit.
- **Ce qui compte comme « joint »** : Matomo a répondu 2xx à un envoi. Une réponse d'erreur
  (site inconnu, 4xx/5xx) compte comme **non joint**.

### 5.3 « Suivi si possible » et « Sans suivi »

- **Suivi si possible** : comportement de la 0.1.1. Les statistiques sont envoyées si Matomo
  répond ; sinon elles sont gardées puis abandonnées au bout d'1 h, sans aucun blocage.
- **Sans suivi** : aucun envoi, aucune file, aucun identifiant visiteur créé.

### 5.4 Journal et statistiques

- Le journal indique le type de clé et son id (`Clé k-7f3a9c · suivi obligatoire`), mais
  **jamais** l'adresse Matomo ni la clé elle-même.
- La 0.1.1 prévoyait un Matomo défini à la compilation (`MATOMO_URL`…), un bloc `matomo` dans
  `config.yaml` et la préférence `statistics`. Tout cela est **supprimé** : c'est la clé qui
  fournit Matomo.

## 6. Développement et tests

- **Clés de dev** : une paire Ed25519 « dev » est commitée (`scripts/cles-dev/`). Les builds
  de dev (`npm start`) et les tests acceptent les clés publiques prod **et** dev ; l'exe
  diffusé (`npm run dist` en CI) n'accepte **que** les clés prod.
- **Tests unitaires** : signature, altération d'un octet, `kid` inconnu, révocation, version
  future, déchiffrement, validation.
- **Tests de bout en bout** :
  - premier lancement → écran d'activation → activation ;
  - suivi obligatoire avec un faux Matomo arrêté → blocage → faux Matomo relancé → déblocage ;
  - aucune fuite de l'adresse Matomo dans la config, le journal ou l'interface.

## 6 bis. Préconfiguration des sources (ajout)

- La clé peut porter une préconfiguration `preset` : `connections` et `profiles` au format
  de `config.yaml`, **sans aucun secret**.
- L'outil la lit depuis un fichier YAML (`--preconfig`), la valide et refuse tout champ qui
  ressemble à un secret. Si l'id manque, il le calcule de façon stable à partir du type et
  du nom.
- À chaque saisie d'une clé, l'application ajoute à `config.yaml` les connexions absentes
  (sont considérées présentes : même id, même URL ou broker commun) et les profils dont le
  `match` n'existe pas. Les données de l'utilisateur priment. Rien n'est réappliqué aux
  démarrages suivants.
- Le contenu de la clé est compressé (deflate) avant chiffrement.

## 7. Limites (à accepter explicitement)

Ocekour est une application locale, sans serveur de licence. Une personne **déterminée**
peut donc :

- **lire l'adresse Matomo** : elle passe de toute façon en clair dans le réseau (DNS, proxy),
  et la clé d'obfuscation est dans l'exe. Le chiffrement empêche la lecture _facile_, pas la
  lecture _déterminée_ ;
- **contourner le blocage** en modifiant le JavaScript de l'exe (non signé) ou en redirigeant
  l'adresse Matomo vers un faux serveur.

En revanche, **personne ne peut fabriquer une clé ni en changer le type** sans la clé privée.
Une clé qui circule peut être **révoquée** dans la version suivante. C'est cohérent avec
l'objectif d'**incitation**.

## 8. Questions ouvertes

1. Version : **0.2.0** pour ce changement, qui bloque l'application ? On publierait alors la
   0.1.1 (journal + Matomo) telle quelle d'ici là, ou on la fusionne dans la 0.2.0.
2. « Suivi obligatoire » : le délai de **15 min** sans envoi réussi avant blocage te va-t-il ?
   Veux-tu aussi un blocage **immédiat** au démarrage (proposé) ?
3. Clé d'obfuscation dans un **secret GitHub** (proposé) ou simplement dans le code, si le
   dépôt reste privé ?
4. Faut-il une clé « **universelle** » à diffuser à toute la société (un seul id), ou une clé
   **par équipe** (id distincts, meilleure lecture dans Matomo) ?
5. Qui génère les clés : toi seul (clé privée sur ton poste) ou plusieurs personnes (clé
   privée dans un coffre partagé) ?
