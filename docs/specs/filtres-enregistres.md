# Spec — Filtres enregistrés & encart rétractable (1.1.0)

> Statut : **implémentée en 1.1.0** · Guide utilisateur : [filtres.md](../guide/filtres.md)
> · Prolonge l'[encart Filtres 1.0.0](filtres.md) (règles masquer/garder par champ).

## 1. Objectif

Depuis la 1.0.0, chaque onglet a un **encart Filtres** : une liste de règles (masquer / garder
seul) mémorisée **par onglet** (localStorage). Trois manques à combler :

1. **Rétractable** — l'encart, quand il est ouvert, pousse la liste des logs vers le bas. On veut
   pouvoir le **replier** (et borner sa hauteur) pour garder les logs visibles.
2. **Nommables & sauvegardables** — pouvoir donner un **nom** à un jeu de règles et l'**enregistrer**.
3. **Réutilisables entre pods** — un jeu enregistré doit être **disponible dans tous les onglets**
   (tous les pods / flux / assemblages), pas seulement là où il a été créé.

Vocabulaire : un **filtre enregistré** (ou **preset**) = un **nom** + une **liste de règles**.
Les règles actives d'un onglet restent, elles, une notion **par onglet** (comme aujourd'hui).

## 2. Modèle

Un filtre enregistré réutilise le modèle de règle de la 1.0.0, **sans l'`id`** (l'`id` est propre
à une instance d'onglet ; il est régénéré au chargement).

```ts
// shared/config.ts — stocké dans config.yaml
interface StoredFilterRule {
  enabled: boolean;      // défaut true
  field: string;         // 'path', 'http.path', '@message', '@line'…
  mode: 'exact' | 'contains' | 'regex';   // défaut 'contains'
  value: string;
  caseSensitive: boolean; // défaut false
  action: 'hide' | 'only'; // défaut 'hide'
}
interface FilterPreset {
  name: string;               // unique (clé)
  rules: StoredFilterRule[];
}
// ConfigSchema : filterPresets: FilterPreset[]  (défaut [])
```

- **Clé = `name`.** Enregistrer sous un nom déjà pris **remplace** ce filtre (mise à jour).
- Les filtres enregistrés vivent dans **`config.yaml`** (source de vérité unique, éditable à la
  main, partagée par tous les onglets et conservée entre deux lancements). Les **règles actives
  d'un onglet** restent dans la session (localStorage), inchangé.

## 3. Persistance (main / config.yaml)

Nouveau module `electron/filter-presets.ts`, sur le modèle de `editProfile` (édition en place du
document YAML, commentaires conservés, refus si le fichier est invalide). Une opération par appel :

- `save(name, rules)` : crée, ou **remplace** le filtre de même nom (tri par nom).
- `delete(name)` : supprime le filtre.
- `rename(from, to)` : renomme (refusé si `to` existe déjà et diffère de `from`).

IPC exposés (preload + `OcekourApi.config`) :

```ts
saveFilterPreset(name: string, rules: StoredFilterRule[]): Promise<ConfigState>;
deleteFilterPreset(name: string): Promise<ConfigState>;
renameFilterPreset(name: string, to: string): Promise<ConfigState>;
```

## 4. Encart rétractable

- Le bouton **Filtres** (barre de l'onglet) **replie / déplie** l'encart (chevron ▸ / ▾). La
  pastille du nombre de règles actives reste visible même replié.
- Ouvert, l'encart borne sa hauteur (`max-h`, défilement interne) : beaucoup de règles ou de
  filtres enregistrés ne chassent plus les logs de l'écran.

## 5. Barre « Filtres enregistrés » (dans l'encart)

Une ligne en tête de l'encart :

- **Sélecteur** des filtres enregistrés (`— aucun —` + un item par filtre).
- **Charger** : remplace les règles de l'onglet par celles du filtre choisi (ids régénérés).
  (Charger n'est pas « lier » : modifier ensuite les règles de l'onglet ne touche pas le filtre
  enregistré tant qu'on ne ré-enregistre pas.)
- **Supprimer** : efface le filtre choisi de `config.yaml`.
- **Champ nom + Enregistrer** : enregistre les **règles actives de l'onglet** sous le nom saisi
  (pré-rempli avec le nom chargé, pour mettre à jour). Nom vide ou sans règle : bouton inactif.

Le reste de l'encart (CRUD de règles, pastille, erreurs de regex, datalist des colonnes) est
inchangé (1.0.0).

## 6. Portée

- **Appliquent** : flux simples et assemblages (même pipeline `LogRows` / `RowFilter`, inchangé).
- **Export / téléchargement filtré** : utilisent les règles **actives** de l'onglet (inchangé) —
  un filtre enregistré n'agit qu'une fois **chargé** dans l'onglet.
- **Partage par lien** : porte toujours les règles actives de l'onglet (déjà le cas depuis 1.0.0 ?
  non — le lien **ne porte pas** les règles ; inchangé), et **ne porte pas** le catalogue de
  filtres enregistrés (local à la machine via `config.yaml`).

## 7. Découpage (tout en 1.1.0)

1. Schéma `FilterPreset` / `StoredFilterRule` dans `shared/config.ts` + types dans `shared/api.ts`.
2. `electron/filter-presets.ts` (save / delete / rename) + IPC `config:*FilterPreset` + preload + tests.
3. `ConfigService` : `presets` + `saveFilterPreset` / `deleteFilterPreset` / `renameFilterPreset`.
4. `log-view` : encart rétractable (hauteur bornée) + barre « Filtres enregistrés » (charger /
   enregistrer / supprimer).
5. Guide, CHANGELOG, version 1.1.0, CI.

## 8. Choix retenus

- **8.1** Nom = clé ; ré-enregistrer sous un nom existant **remplace** (pas de doublon). (reco)
- **8.2** Stockage dans **`config.yaml`** (et non un fichier séparé) : éditable, versionnable,
  cohérent avec profils/favoris. (reco)
- **8.3** « Charger » **copie** les règles dans l'onglet (pas de lien vivant) : simple et prévisible.
  Un lien vivant (l'onglet suit le filtre) est **hors périmètre** (plus tard, si besoin).
- **8.4** Le catalogue de filtres **n'est pas** inclus dans le partage par lien (local machine). (reco)
