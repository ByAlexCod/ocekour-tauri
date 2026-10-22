# Prise en main

[← Sommaire](README.md)

## L'écran

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Ocekour   Logs  Paramètres                         Système Clair Sombre  │
├───────────────┬──────────────────────────────────────────────────────────┤
│ Filtrer… ⟳ 🔗 +│ [onglet 1] [onglet 2] …                                  │
│ ★ Favoris     │ Barre d'options (mode, lignes, depuis, lien, export…)    │
│ ▾ RANCHER Prod│ Recherche · niveaux                                      │
│   ▾ cluster   │ ┌────────────────────────────┬─────────────────────────┐ │
│     ▾ ns      │ │ Lignes de log              │ Détail de la ligne      │ │
│       ▸ pod   │ │                            │ sélectionnée            │ │
│ ▾ KAFKA …     │ └────────────────────────────┴─────────────────────────┘ │
│ ▾ S3 …        │                                                          │
└───────────────┴──────────────────────────────────────────────────────────┘
```

- **Barre latérale** : vos connexions, sous forme d'arbre. Chaque connexion porte un badge
  RANCHER, KAFKA ou S3. Le champ **Filtrer** cherche parmi les éléments déjà chargés.
  Pour élargir ou réduire la barre, **glissez son bord droit**. Un double-clic rétablit la
  largeur par défaut, et les flèches ← → fonctionnent quand le bord a le focus. La largeur
  est mémorisée.
- Boutons en haut de la barre latérale :
  - **⟳** rafraîchit l'arbre ;
  - **🔗** ouvre le lien Ocekour présent dans le presse-papiers ;
  - **+** crée une nouvelle connexion.
- **Onglets** : un par conteneur, topic ou bucket ouvert. Ils sont réordonnables par
  glisser-déposer et rouverts au démarrage (réglable dans **Paramètres**).
- **Thème** : Système, Clair ou Sombre, en haut à droite.

## Ouvrir quelque chose

| Action                           | Effet                       |
| -------------------------------- | --------------------------- |
| Clic                             | Ouvre dans l'onglet courant |
| **Ctrl+clic** ou **clic milieu** | Ouvre dans un nouvel onglet |
| Clic sur un élément déjà ouvert  | Affiche son onglet          |

Un point de couleur sur l'onglet indique l'état du flux : vert = en direct, orange =
connexion ou reconnexion, gris = terminé, rouge = erreur. Une pastille rouge compte les
**nouvelles erreurs** arrivées pendant que l'onglet était caché.

## Favoris

Survolez un namespace ou un pod, puis cliquez sur **☆** pour l'épingler. Les favoris
apparaissent tout en haut, dans la section **★ Favoris**. Un workload épinglé se déplie
directement sur ses pods **actuels**, même après un redéploiement. Les topics et les
buckets peuvent aussi être épinglés.

## Raccourcis clavier

| Raccourci                               | Effet                                                                 |
| --------------------------------------- | --------------------------------------------------------------------- |
| **Ctrl+F**                              | Aller au champ de recherche                                           |
| **Entrée** / **F3** · **Maj+F3**        | Correspondance suivante / précédente (mode _Surligner_)               |
| **Échap**                               | Vider la recherche (dans le champ) ; fermer le détail (dans la liste) |
| **↑ ↓ PgPréc PgSuiv Début Fin**         | Naviguer dans les lignes (la liste doit avoir le focus)               |
| **Ctrl+W**                              | Fermer l'onglet                                                       |
| **Ctrl+Tab** / **Ctrl+Maj+Tab**         | Onglet suivant / précédent (aussi **Ctrl+PgSuiv** / **Ctrl+PgPréc**)  |
| **Ctrl+V** (hors d'un champ)            | Ouvrir le lien Ocekour collé                                          |
| **Ctrl+S**                              | Enregistrer `config.yaml` (dans l'éditeur des Paramètres)             |
| **F2** / **Suppr** / **Retour arrière** | Explorateur S3 : renommer / supprimer / dossier parent                |
