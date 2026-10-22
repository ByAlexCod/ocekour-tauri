# Clés d'activation — guide administrateur

> Pour la personne qui **émet** les clés. Côté utilisateur, voir
> [docs/guide/installation.md](guide/installation.md#activation). Conception :
> [docs/specs/cle-activation.md](specs/cle-activation.md).

Ocekour ne démarre qu'avec une clé d'activation. La clé fixe le **type de suivi** et porte la
**configuration Matomo** : il n'y en a nulle part ailleurs (ni dans `config.yaml`, ni dans
l'exe).

| Type (`--mode`)               | Statistiques Matomo | Si Matomo est injoignable                                                                                          |
| ----------------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `none`, sans suivi            | aucune              | sans objet                                                                                                         |
| `optional`, suivi si possible | envoyées            | l'application fonctionne ; les statistiques sont gardées une heure puis abandonnées                                |
| `required`, suivi obligatoire | envoyées            | **bloquée** au démarrage tant que Matomo n'a pas répondu, et en cours d'utilisation après 15 min sans envoi réussi |

Les clés **n'expirent pas**. Une clé diffusée par erreur se **révoque** dans la version
suivante (voir plus bas).

## Mise en place (une fois)

Prérequis : Node ≥ 22.18, le dépôt cloné.

```bash
ocekour-headless cle init --kid prod1 --dossier ~/ocekour-prive
```

- **Clé privée de signature** : écrite dans `~/ocekour-prive/ocekour-prod1.pem`. À ranger
  dans un coffre de mots de passe. Elle ne doit **jamais** être commitée ni diffusée :
  quiconque l'a peut fabriquer des clés.
- **Clé publique** : ajoutée dans `crates/core/src/licence/keys.rs`. **Commitez ce fichier.**
- **Clé d'obfuscation** : affichée à l'écran. C'est elle qui chiffre la configuration Matomo
  dans les clés. Enregistrez-la :
  - dans le secret GitHub **`LICENCE_OBFUSCATION`** (_Settings → Secrets and variables →
    Actions → New repository secret_) ;
  - dans votre coffre, car elle est nécessaire pour générer ou relire des clés.

Sans clé publique de production ou sans ce secret, la CI **refuse de produire l'exe**, avec
un message explicite. L'exe diffusé n'accepte que les clés de production.

## Générer une clé

```bash
export LICENCE_OBFUSCATION='…'        # depuis votre coffre

# Suivi obligatoire pour l'équipe Paiement
ocekour-headless cle generer --kid prod1 --prive ~/ocekour-prive/ocekour-prod1.pem \
  --label "Équipe Paiement" --mode required \
  --matomo-url https://matomo.intra/ --site-id 12 --dim-version 1 --dim-cle 2

# Sans suivi (poste isolé)
ocekour-headless cle generer --kid prod1 --prive ~/ocekour-prive/ocekour-prod1.pem \
  --label "Poste labo" --mode none
```

La clé s'affiche sur une seule ligne (`OCEKOUR1-prod1.…`, environ 400 caractères). Envoyez-la à
l'équipe, par exemple en message privé Rocket.Chat. L'utilisateur la colle au premier
lancement.

| Option                      | Rôle                                                                                                                                                      |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--label`                   | À qui la clé est remise. Affiché dans l'application, **jamais envoyé à Matomo**                                                                           |
| `--mode`                    | `none`, `optional` ou `required`                                                                                                                          |
| `--matomo-url`, `--site-id` | Matomo et site qui reçoivent les statistiques (obligatoires avec suivi)                                                                                   |
| `--dim-version`             | Index d'une dimension personnalisée Matomo (portée _visite_) qui reçoit la version d'Ocekour                                                              |
| `--dim-cle`                 | Index d'une dimension personnalisée (portée _visite_) qui reçoit l'**id de la clé** : l'usage se lit ainsi par clé, donc par équipe                       |
| `--matomo-insecure`         | Ne pas vérifier le certificat HTTPS de Matomo (déconseillé)                                                                                               |
| `--preconfig fichier.yaml`  | Préconfiguration : connexions, profils et URL des services (GitLab, Jira, Confluence, Rocket.Chat) ajoutés à `config.yaml` à l'activation (voir plus bas) |

**Tenez un registre** des clés émises, hors du dépôt : id, libellé, type, date, destinataire.

Côté Matomo, créez un site « Ocekour » et, si vous le souhaitez, deux dimensions
personnalisées de portée _visite_ : « Version » et « Clé ». Leurs index se passent en
`--dim-version` et `--dim-cle`.

## Préconfigurer les sources

Une clé peut porter les **connexions** (Rancher, Kafka, S3) et les **profils** de l'équipe.
À l'activation, Ocekour les ajoute à `config.yaml` : l'utilisateur n'a plus qu'à saisir **ses**
identifiants via « Mettre à jour les identifiants » dans l'arbre.

Le fichier est au **format de `config.yaml`**, avec seulement les sections `connections` et
`profiles`. Exemple complet : [docs/exemples/preconfiguration.yaml](exemples/preconfiguration.yaml).

```yaml
connections:
  - type: rancher
    name: Rancher prod
    url: https://rancher.intra
    clusters: [{ id: c-m-abcd1234, name: prod }]
  - type: kafka
    name: Kafka prod
    brokers: [kafka1.intra:9092, kafka2.intra:9092]
    registry: { url: http://schema-registry.intra:8081 }
profiles:
  - match: 'prod/*/api-*'
    columns: [traceId]
```

```bash
ocekour-headless cle generer … --label "Équipe Paiement" --mode optional … \
  --preconfig preconfig-paiement.yaml
```

- **Aucun secret** : l'outil refuse tout champ dont le nom évoque un secret (`token`,
  `password`, `secret`…). Une clé circule ; chacun garde ses identifiants.
- **Validation** : l'outil vérifie chaque connexion comme l'application, et le message
  indique l'endroit fautif (ex. `connections[0].url`).
- **Id stables** : `id` est facultatif. S'il manque, il est calculé à partir du type et du
  nom, donc identique d'une clé à l'autre.
- **Pas de doublon** : une connexion n'est pas ajoutée si le poste en a déjà une avec le même
  id, la même URL (Rancher, S3) ou un broker en commun (Kafka). Un profil n'est pas ajouté si
  son `match` existe déjà. **La configuration de l'utilisateur prime.**
- **Services (0.3.0)** : section `services`, avec seulement `url` et `tls` par outil
  (`gitlab`, `jira`, `confluence`, `rocketchat`). Un outil déjà configuré sur le poste n'est
  pas modifié. La clé n'active jamais l'écriture : chacun crée son jeton et décide. Une
  version d'Ocekour antérieure à 0.3.0 ignore cette section.
- **Quand** : la préconfiguration est appliquée à **chaque saisie d'une clé**, au premier
  lancement ou via « Changer de clé », mais pas aux démarrages suivants. Une connexion
  supprimée par l'utilisateur ne revient donc pas d'elle-même.
- **Taille** : le contenu est compressé. Une clé avec une dizaine de connexions fait environ
  1 000 caractères.

## Relire une clé (support)

```bash
LICENCE_OBFUSCATION='…' ocekour-headless cle lire 'OCEKOUR1-prod1.…'
```

La commande affiche le contenu (type, Matomo, id) si la signature est valide, sinon la raison
du refus.

## Révoquer une clé

1. Ajoutez son id dans `REVOKED_KEY_IDS` (`crates/core/src/licence/keys.rs`).
2. Publiez une nouvelle version.

Sur les postes mis à jour, la clé est refusée et l'écran d'activation réapparaît avec le
message « clé révoquée ».

## Changer de clé de signature

Lancez `init` avec un nouveau `--kid` (par ex. `prod2`). L'exe accepte alors les deux clés
publiques : les clés déjà émises restent valides, et les nouvelles sont signées par `prod2`.
Pour invalider toutes les clés d'un ancien `kid`, retirez sa clé publique de `cles.ts`.

## Développement

- `cargo tauri dev` et les tests acceptent aussi les clés **de dev**, dont la clé privée
  `scripts/cles-dev/dev.pem` est publique.
- Pour en générer une : `ocekour-headless cle generer --kid dev --label Dev --mode none`.
- L'exe de la CI (`OCEKOUR_RELEASE=1`) les **refuse**.

## Limites

L'application est locale et l'exe n'est pas signé : une personne déterminée peut lire
l'adresse Matomo (elle circule de toute façon sur le réseau) ou modifier l'exe pour
contourner le blocage. Le dispositif **incite** au reporting et empêche de **fabriquer** ou
de **modifier** une clé ; ce n'est pas une protection anti-piratage.
