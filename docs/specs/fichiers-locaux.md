# Spec — Logs depuis un fichier local (2.1.0)

> Statut : **proposition** · S'appuie sur les onglets de logs (`streams.open`), l'analyse des
> lignes (parser, profils, filtres), l'assemblage et le serveur MCP.
> Socle commun avec la [sonde](sonde.md) : la lecture de fichier (`streams/file.rs`) est la même
> en local et sur une machine distante.

## 1. Objectif

Aujourd'hui, un onglet de logs lit un **conteneur Rancher** ou un **topic Kafka**. On veut
ouvrir aussi un **fichier de logs présent sur le poste** (log applicatif récupéré, export,
log d'un service Windows, fichier d'un partage réseau…) avec **les mêmes outils** : niveaux,
recherche, filtres, profils, détail JSON, export, assemblage.

Principes :

- **Un fichier est un flux comme un autre** : nouvelle cible `file`, ouverte par
  `streams.open` ; tout l'aval (tampon, parser, vue, filtres) est réutilisé sans changement.
- **Suivi possible** (`tail -f`) : si le fichier grossit, les lignes arrivent en direct.
- **Lecture seule** : Ocekour n'écrit, ne verrouille ni ne déplace jamais le fichier.
- **Rien n'est exposé sans action de l'utilisateur** : seuls les fichiers qu'il a choisis
  sont lus (y compris par les agents, §8).

Hors périmètre 2.1 : dossiers entiers et motifs (`*.log`) en local (voir la sonde, §4) ;
journaux système (journald, Observateur d'événements Windows) ; écriture ou édition.

## 2. Parcours utilisateur

1. **Ouvrir** :
   - bouton **« Ouvrir un fichier… »** dans le panneau des sources (icône 📄) et raccourci
     **Ctrl+O** ;
   - **glisser-déposer** un ou plusieurs fichiers sur la fenêtre (un onglet par fichier ;
     Ctrl enfoncé ou plusieurs fichiers → proposer « Ouvrir en assemblage ») ;
   - **ligne de commande** : `ocekour.exe C:\logs\app.log` ouvre le fichier dans la fenêtre
     existante (instance unique : l'argument est transmis à la fenêtre déjà ouverte).
2. L'onglet s'ouvre comme un onglet de logs : titre = nom du fichier, info-bulle = chemin
   complet, fil d'Ariane `Fichiers locaux / dossier / nom`.
3. Options de l'onglet : **Suivi / Historique**, **Lignes** (1 000 par défaut, « Tout »),
   **Depuis** (sur l'horodatage des lignes, §5). « Précédent » n'a pas de sens : masqué.
4. Le panneau des sources gagne une section **« Fichiers locaux »** : fichiers **récents**
   (10 derniers) et **favoris** (étoile, comme les pods). Un fichier disparu est grisé
   (« introuvable ») et peut être retiré.

## 3. Modèle

```ts
/** Fichier de logs : local (connectionId absent) ou exposé par une sonde (sonde.md). */
export interface FileTarget {
  kind: 'file';
  /** Absent = fichier du poste ; sinon connexion de type `sonde`. */
  connectionId?: string;
  /** Local : chemin absolu. Sonde : id de la source puis, pour un motif, le fichier choisi. */
  path: string[];
  /** Nom affiché (nom du fichier, ou nom de la source de la sonde). */
  name: string;
}
export type StreamTarget = ContainerTarget | TopicTarget | FileTarget;
```

- `targetKey` = `file/<connectionId|local>/<chemin>`.
- Assemblage : `AssemblyMember.target` accepte `FileTarget` (fichier local + pods + topics dans
  une même vue triée).
- `config.yaml` : récents et favoris **ne sont pas** dans `config.yaml` (chemins propres au
  poste, pas à partager) → `fichiers-recents.json` dans le dossier de données. _(reco)_

Côté cœur, `StreamManager::open` reçoit une **source locale** : connexion réservée `local`
(id fixe, jamais écrite dans `config.yaml`), chemin = `[chemin absolu]`.

## 4. Lecture (`crates/core/src/streams/file.rs`)

| Besoin              | Comportement                                                                                                                                                         |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| N dernières lignes  | Lecture **depuis la fin** par blocs de 64 Ko jusqu'à N fins de ligne : instantané même sur un fichier de plusieurs Go.                                               |
| « Tout »            | Lecture du début, bornée par la taille du tampon de l'onglet (`bufferSize`) : au-delà, les plus anciennes sont retirées (comportement actuel).                       |
| Suivi               | Après l'historique, on garde la position et on lit ce qui est ajouté : notification système (`notify`) + vérification toutes les 500 ms (partages réseau, SMB).       |
| Rotation            | Fichier remplacé (identité changée : inode / id de fichier Windows) ou tronqué (taille < position) : notice « Fichier renouvelé » puis lecture depuis le début.       |
| Fichier supprimé    | Statut `reconnecting` (« fichier absent, en attente »), reprise s'il réapparaît ; `ended` en mode historique.                                                         |
| Encodage            | UTF-8 (remplacement des octets invalides), **UTF-16 LE/BE détecté par BOM** (logs Windows, PowerShell), Latin-1 au choix dans le menu de l'onglet. _(reco)_          |
| Fins de ligne       | `\n` et `\r\n` ; dernière ligne sans fin gardée en attente en suivi (`LineSplitter` existant).                                                                       |
| Lignes très longues | Coupées à 1 Mo avec la mention « … (tronqué) » (protection mémoire).                                                                                                 |
| Compressé           | `.gz` lu en historique seulement (décompression en flux) ; suivi désactivé. `.zip` hors périmètre. _(reco)_                                                          |
| Partage             | Ouverture en lecture **partagée** (Windows : `FILE_SHARE_READ \| WRITE \| DELETE`) pour ne jamais gêner l'application qui écrit le log ni sa rotation.                 |

Horodatage d'une ligne : celui reconnu par le parser existant (ISO 8601, `yyyy-MM-dd HH:mm:ss`,
JSON `@timestamp`…) ; à défaut, **pas d'horodatage** (la colonne heure reste vide, la ligne
garde sa position). Lignes multi-lignes (stack traces) : regroupées comme aujourd'hui.

## 5. « Depuis » sur un fichier

Un fichier n'a pas d'API « depuis N secondes » : on lit depuis la fin et on s'arrête au premier
bloc dont la première ligne datée est plus ancienne que la limite. Les lignes non datées suivent
la ligne datée qui les précède. Si aucune ligne n'est datée, « Depuis » est désactivé avec
l'info-bulle « horodatage non reconnu ».

## 6. Partage, export, profils

- **Lien de partage** : désactivé pour un fichier local (chemin propre au poste), avec
  l'info-bulle « fichier local : utilisez Exporter ». Fichier de sonde : partageable (§ sonde).
- **Exporter / Télécharger** : comme aujourd'hui.
- **Profils** (colonnes, niveaux) : clé `fichier/<nom sans chiffres de rotation>` (ex.
  `app.2026-10-22.log` → `fichier/app.log`) pour retrouver le profil d'un jour à l'autre.

## 7. Performance

- Ouverture d'un fichier de 2 Go en « 1 000 dernières lignes » : **< 200 ms**.
- Suivi : latence d'affichage < 1 s ; lots regroupés comme les flux Rancher.
- « Tout » sur un gros fichier : lecture en flux, l'interface reste fluide (lots de 5 000
  lignes) ; avertissement si le fichier dépasse 10 × `bufferSize` lignes estimées.

## 8. Agents (MCP)

Lire n'importe quel fichier du poste serait un accès bien plus large que les logs : les agents
n'ont accès **qu'aux fichiers que l'utilisateur a déclarés** pour eux.

- Paramètres → **« Fichiers visibles par les agents »** : liste de chemins ou de motifs
  (`C:\app\logs\*.log`), vide par défaut, dans `config.yaml` (`fichiers.agents`).
- `logs_lire` / `logs_exporter` acceptent `{ "fichier": "<chemin>" }` s'il correspond à la
  liste (chemin canonique, liens symboliques résolus avant la vérification) ; sinon refus
  explicite (« fichier non autorisé : ajoutez-le dans Paramètres → Fichiers »).
- Nouvel outil `fichiers_lister` : fichiers autorisés existants, taille, date de modification.

## 9. Tests

- **Cœur** (`streams/file.rs`) : N dernières lignes sur un fichier > 10 Mo (lecture depuis la
  fin), « Tout », suivi (ajouts en direct), rotation par renommage, troncature, suppression puis
  recréation, CRLF, UTF-16 avec BOM, octets invalides, ligne de 5 Mo, `.gz`, « Depuis ».
- **MCP** : fichier autorisé lu, fichier hors liste refusé, contournement par `..` et par lien
  symbolique refusé.
- **Interface** (vitest) : cible `file` (clé, onglet, restauration de session, assemblage),
  récents/favoris, lien de partage désactivé.
- **Application** : glisser-déposer et `ocekour.exe <fichier>` (test IPC Tauri).

## 10. Questions ouvertes

1. Dossiers et motifs en local dès la 2.1 (« suivre le plus récent `app-*.log` ») ou seulement
   via la sonde ?
2. Accès des agents aux fichiers : liste blanche (§8, recommandé), ou pas d'accès du tout en 2.1 ?
3. Encodages à prendre en charge en plus d'UTF-8 / UTF-16 (Windows-1252 ?).
