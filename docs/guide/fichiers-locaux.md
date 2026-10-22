# Fichiers locaux : lire les logs du poste

[← Sommaire](README.md)

Ocekour lit aussi les fichiers de logs de votre poste (`.log`, `.txt`, `.out`, `.json`…), avec
les mêmes outils que pour un pod : suivi en direct, recherche, niveaux, filtres, export,
assemblage.

## Ouvrir un fichier

- **Ctrl+O**, ou **Ouvrir…** dans la section **📄 Fichiers locaux** du panneau des sources ;
- **glisser-déposer** un ou plusieurs fichiers sur la fenêtre (un onglet par fichier) ;
- **ligne de commande** : `Ocekour.exe C:\logs\app.log` ; si Ocekour est déjà ouvert, le
  fichier s'ouvre dans la fenêtre existante (pratique avec « Ouvrir avec… » de l'Explorateur).

L'onglet propose **Suivi / Historique**, **Lignes** et **Depuis** comme un pod. « Précédent »
n'existe pas pour un fichier.

## Récents et favoris

La section **Fichiers locaux** garde les fichiers ouverts récemment, et les **favoris** (☆ au
survol) en tête. Un fichier disparu est grisé (« introuvable ») ; ✕ le retire de la liste.
Cette liste est propre au poste (elle n'est pas dans `config.yaml`).

## Motifs : suivre une famille de fichiers

Beaucoup d'applications écrivent un fichier par jour ou par taille (`app-2026-10-22.log`,
`app.log.1`…). **Motif…** dans la section permet de saisir un chemin dont le **nom** contient
des jokers :

```
C:\app\logs\app-*.log
D:\batch\batch-????-??-??.log
```

- Jokers `*`, `?` et `[0-9]` dans le nom du fichier seulement (le dossier est fixe).
- Ouvrir le motif suit **le fichier le plus récent** ; à la rotation, le suivi **bascule** sur
  le nouveau fichier (notice « Nouveau fichier : … » dans le flux).
- ▸ déplie le motif : ses fichiers, du plus récent au plus ancien (30 au plus), avec taille et
  âge. Un clic ouvre ce fichier précis.

## Encodage

Par défaut, Ocekour détecte l'UTF-8 et les BOM UTF-16. Pour un vieux fichier Windows aux
accents illisibles, choisissez **Encodage → Windows-1252** (ou Latin-1, UTF-16) dans la barre de
l'onglet : le choix est mémorisé pour ce fichier.

## Partage, export, profils

- **Lien** est désactivé pour un fichier local : le chemin n'a de sens que sur votre poste.
  Utilisez **Exporter…** pour transmettre les lignes.
- Les **profils** (colonnes, niveaux) d'un fichier ont pour clé `fichier/<nom>` sans date ni
  numéro de rotation : `app.2026-10-22.log` et `app.2026-10-23.log` partagent `fichier/app.log`.

## Pour les agents (MCP)

Les agents **n'ont accès à aucun fichier du poste par défaut**. Pour leur en ouvrir :
**Paramètres → Fichiers pour les agents**, puis ajoutez des chemins ou des motifs
(`C:\app\logs\*.log`). Ils sont enregistrés dans `config.yaml` :

```yaml
fichiers:
  agents:
    - C:\app\logs\*.log
    - C:\outil\trace.txt
```

Les agents les voient avec `fichiers_lister` et les lisent avec `logs_lire` (connexion `local`).
Tout autre chemin est refusé.
