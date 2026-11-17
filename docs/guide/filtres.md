# Filtres de logs

[← Sommaire](README.md)

Depuis la 1.0.0, chaque onglet de logs a un encart **Filtres** : des **règles** cumulables qui
**masquent** (ou gardent seules) des lignes selon la valeur d'un **champ**. Utile, par exemple,
pour cacher tous les logs d'un certain `path`.

## Ouvrir et replier l'encart

Bouton **Filtres** dans la barre de l'onglet (une pastille indique le nombre de règles actives).
Il **déplie / replie** la liste des règles sous la barre (chevron ▸ / ▾) ; **▴ Replier**, en tête
de l'encart, le replie aussi. Les règles restent appliquées une fois l'encart replié.

- **Replié**, l'encart tient sur **une seule ligne** qui résume les règles (« Garder level = error »,
  « Masquer path contient /health »…) : les logs gardent toute la place. Cliquer une règle de ce
  résumé l'**active / la désactive** (barrée quand elle est désactivée) ; **▸ Filtres** redéplie.
- **Déplié**, l'encart est limité au tiers de la hauteur et défile si les règles sont nombreuses.
- Ouvert ou replié : mémorisé pour les autres onglets et le lancement suivant. Charger un filtre
  enregistré ne le déplie plus.

## Filtres enregistrés (réutilisables entre pods) — depuis la 1.1.0

En tête de l'encart, la ligne **Filtre enregistré** permet de **nommer**, **enregistrer** et
**réutiliser** un jeu de règles dans **tous les onglets** (tous les pods, flux et assemblages) :

| Élément | Rôle |
| ------- | ---- |
| Sélecteur | Choisir un filtre enregistré : ses règles (et sa combinaison ET / OU) **remplacent** celles de l'onglet (copie) |
| **+ Combiner avec…** | **Ajoute** les règles d'un filtre enregistré à celles déjà là : plusieurs filtres enregistrés combinés dans un onglet |
| Supprimer | Efface le filtre enregistré sélectionné |
| Nom + **Enregistrer** | Enregistre les **règles actuelles** de l'onglet et leur combinaison (ET / OU) sous ce nom (un nom déjà pris est **remplacé**) |

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

## Comment les règles se combinent (OU / ET)

En tête de l'encart, **Combiner : OU | ET** choisit comment les règles d'une même action se
combinent (mémorisé par onglet et dans les filtres enregistrés) :

| | **OU** (par défaut) | **ET** |
| --- | --- | --- |
| Garder seul | la ligne est gardée si elle correspond à **l'une** des règles | gardée seulement si elle correspond à **toutes** |
| Masquer | la ligne est masquée dès qu'**une** règle correspond | masquée seulement si **toutes** correspondent |

Une ligne est affichée si, après les niveaux et la recherche, elle passe les règles **Garder
seul** (s'il y en a), puis n'est pas masquée par les règles **Masquer** : la liste noire l'emporte.
Un champ **absent** d'une ligne ne correspond pas (une règle *Masquer* ne la cache donc pas).

Exemples :

- *Garder seul* `level` = `error` **ET** *Garder seul* `path` contient `/api` → seulement les erreurs
  des appels `/api` (en OU : toutes les erreurs, plus tous les appels `/api`).
- *Masquer* `path` = `/health` **ET** *Masquer* `level` = `info` → cache les sondes de santé
  réussies, mais garde une erreur sur `/health`.

## Bon à savoir

- Les règles sont **mémorisées par onglet** et rouvertes au démarrage, comme la recherche et les
  niveaux. Elles s'appliquent aussi aux **assemblages**.
- Elles agissent **à l'affichage** : les lignes masquées restent en mémoire (décocher la règle les
  réaffiche aussitôt) et ne sont pas incluses dans **Exporter…**. Le **Télécharger…** filtré les
  applique aussi.
- Les événements d'Ocekour (« Conteneur redémarré ») ne sont jamais masqués.
