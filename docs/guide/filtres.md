# Filtres de logs

[← Sommaire](README.md)

Depuis la 1.0.0, chaque onglet de logs a un encart **Filtres** : des **règles** cumulables qui
**masquent** (ou gardent seules) des lignes selon la valeur d'un **champ**. Utile, par exemple,
pour cacher tous les logs d'un certain `path`.

## Ouvrir et replier l'encart

Bouton **Filtres** dans la barre de l'onglet (une pastille indique le nombre de règles actives).
Il **déplie / replie** la liste des règles sous la barre (chevron ▸ / ▾). Replié, les logs
restent entièrement visibles ; déplié, l'encart borne sa hauteur et défile tout seul si les
règles sont nombreuses.

## Filtres enregistrés (réutilisables entre pods) — depuis la 1.1.0

En tête de l'encart, la ligne **Filtre enregistré** permet de **nommer**, **enregistrer** et
**réutiliser** un jeu de règles dans **tous les onglets** (tous les pods, flux et assemblages) :

| Élément | Rôle |
| ------- | ---- |
| Sélecteur | Choisir un filtre enregistré : ses règles sont **chargées** dans l'onglet courant (copie) |
| Supprimer | Efface le filtre enregistré sélectionné |
| Nom + **Enregistrer** | Enregistre les **règles actuelles** de l'onglet sous ce nom (un nom déjà pris est **remplacé**) |

Les filtres enregistrés sont stockés dans **`config.yaml`** (clé `filterPresets`), donc conservés
entre deux lancements et éditables à la main. **Charger** fait une **copie** : modifier ensuite les
règles de l'onglet ne change pas le filtre enregistré tant qu'on ne le ré-enregistre pas.

## Une règle

| Élément           | Rôle                                                                                   |
| ----------------- | -------------------------------------------------------------------------------------- |
| Case active       | Active / désactive la règle sans la supprimer                                          |
| Masquer / Garder seul | **Masquer** = liste noire (on cache les correspondances) · **Garder seul** = liste blanche (on ne garde qu'elles) |
| Champ             | `path`, `http.path`, `logger`… (chemin de champ JSON, point = profondeur), ou `@message` (le message analysé), ou `@line` (la ligne brute entière) |
| contient / exact / regex | Comment comparer la valeur du champ                                             |
| Valeur            | Texte ou expression régulière (une regex invalide est signalée en rouge)               |
| Aa                | Respecter la casse                                                                     |
| ✕                 | Supprimer la règle                                                                     |

**+ Ajouter une règle** en crée une (par défaut : masquer, champ `path`, « contient »).
**Tout effacer** les enlève toutes.

## Comment les règles se combinent

Une ligne est affichée si, après les niveaux et la recherche :

1. s'il existe au moins une règle **Garder seul** active, la ligne correspond à **l'une** d'elles ;
2. et elle ne correspond à **aucune** règle **Masquer** active.

Autrement dit : les **Garder seul** forment une liste blanche (OU entre elles), les **Masquer**
une liste noire (OU entre elles), et la liste noire l'emporte. Un champ **absent** d'une ligne ne
correspond pas (une règle *Masquer* ne la cache donc pas).

## Bon à savoir

- Les règles sont **mémorisées par onglet** et rouvertes au démarrage, comme la recherche et les
  niveaux. Elles s'appliquent aussi aux **assemblages**.
- Elles agissent **à l'affichage** : les lignes masquées restent en mémoire (décocher la règle les
  réaffiche aussitôt) et ne sont pas incluses dans **Exporter…**. Le **Télécharger…** filtré les
  applique aussi.
- Les événements d'Ocekour (« Conteneur redémarré ») ne sont jamais masqués.
