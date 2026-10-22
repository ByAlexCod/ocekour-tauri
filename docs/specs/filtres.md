# Spec — Filtres de logs (encart de règles) (1.0.0)

> Statut : **implémentée en 1.0.0** · Guide utilisateur : [filtres.md](../guide/filtres.md)
> · S'appuie sur le filtrage existant (recherche + niveaux) et le pipeline `LogRows`
>
> Choix retenus (§8) : champ nommé + `@message`/`@line` (8.1) ; `only` **et** `hide` dès la 1.0.0
> (8.2) ; le **partage par lien ne porte pas encore les règles** (8.3, plus tard) ; presets
> réutilisables **plus tard** (8.4).

## 1. Objectif

Aujourd'hui, un onglet de logs n'a qu'**une** recherche (texte ou regex sur toute la ligne) et le
filtre par **niveaux**. On ajoute un **encart « Filtres »** : une liste de **règles nommées**,
cumulables, qui **masquent** (ou gardent) des lignes selon la valeur d'un **champ**.

Cas d'usage donné : « cacher tous les logs avec un certain `path` » — en **exact** ou en **regex**.

## 2. Modèle d'une règle

```ts
interface LogFilterRule {
  id: string;
  /** Règle active (décochable sans la supprimer). */
  enabled: boolean;
  /** Champ visé : chemin de champ JSON (`path`, `http.path`, `logger`…), `@message`
   *  (le message analysé), ou `@line` (la ligne brute entière). */
  field: string;
  /** Comparaison de la valeur du champ. */
  mode: 'exact' | 'contains' | 'regex';
  value: string;
  caseSensitive: boolean;
  /** `hide` : masquer les lignes qui correspondent ; `only` : ne garder qu'elles. */
  action: 'hide' | 'only';
}
```

- La **valeur du champ** est résolue comme dans l'interface : `@line` → ligne brute ; `@message` →
  message analysé ; sinon `getFieldDeep(json, field)` sur le JSON de la ligne (point = profondeur,
  ex. `http.path`). Une valeur non-chaîne est comparée via sa forme texte.
- Si le champ est **absent** d'une ligne : la règle **ne correspond pas** (donc une règle `hide`
  ne la masque pas ; une règle `only` la masque — voir §3).

## 3. Combinaison

Dans l'ordre, une ligne est affichée si **toutes** ces conditions sont vraies (ET) :

1. Niveaux cochés (inchangé) ;
2. Recherche en mode « filtrer » (inchangé) ;
3. **Règles de filtre** actives :
   - Lignes **gardées** seulement si elles correspondent à **au moins une** règle `only`
     (s'il en existe au moins une active) ;
   - puis **masquées** si elles correspondent à **au moins une** règle `hide`.

Autrement dit : `only` = liste blanche (OU entre elles), `hide` = liste noire (OU entre elles),
la liste noire l'emporte. Les notices d'Ocekour (« Conteneur redémarré ») ne sont jamais masquées.

## 4. Encart « Filtres »

- Un bouton **Filtres** (avec une pastille indiquant le nombre de règles actives) ouvre un **encart**
  sous la barre de l'onglet.
- Chaque règle sur une ligne : case **active**, sélecteur **champ** (saisie libre + raccourcis
  `@message`, `@line` et les colonnes du profil), sélecteur **exact / contient / regex**, champ
  **valeur**, bascule **Aa** (casse), bascule **masquer / garder**, bouton **✕**.
- Boutons **+ Ajouter une règle** et **Tout effacer**. Une regex invalide est signalée sur la règle
  (la règle est ignorée tant qu'elle est invalide).
- **Raccourci depuis une ligne** : le menu d'une valeur (panneau de détail) propose déjà « filtrer
  sur cette valeur » ; on ajoute **« Masquer ce champ = valeur »** qui crée une règle `hide` exacte.

## 5. Portée et persistance

- **Par onglet** : les règles sont mémorisées dans l'état de session (`TabState.rules`), rouvertes
  au démarrage, comme la recherche et les niveaux.
- S'appliquent aux **flux simples et aux assemblages** (même pipeline `LogRows`/`RowFilter`).
- Le **partage par lien** d'un onglet simple inclut les règles (sans secret). _(à confirmer, §8)_
- **Hors périmètre 1.0.0** : des presets de filtres réutilisables entre onglets / enregistrés dans
  `config.yaml` (viendront plus tard).

## 6. Implémentation

- `LogFilterRule` + compilation en prédicats (réutilise `buildMatcher` pour `regex`/`contains`,
  `getFieldDeep` pour la valeur de champ). Un module `log-filters.ts` (testable isolément).
- `RowFilter` gagne `rules?: CompiledRule[]` ; `LogRows.accepts` applique only-liste puis hide-liste
  après niveaux + recherche. Recalcul incrémental inchangé (chaque lot refiltré).
- `log-view` : encart + signaux de règles sur la session ; l'effet de filtre existant inclut les
  règles compilées dans l'appel `setFilter`.
- `AssemblySession.setFilter` fusionne déjà `hiddenSources` ; il transmettra aussi `rules`.

## 7. Découpage (tout en 1.0.0)

1. `log-filters.ts` (modèle, compilation, application) + tests.
2. `RowFilter.rules` + `LogRows.accepts` (only/hide) + tests.
3. Encart « Filtres » dans `log-view` (CRUD de règles, pastille, erreurs de regex).
4. Persistance `TabState.rules` (session) ; assemblages ; raccourci « masquer ce champ ».
5. Guide, CHANGELOG, version, CI.

## 8. Questions ouvertes

- **8.1** Champ visé : chemin de champ **+** `@message`/`@line` (ma reco), ou seulement la ligne
  entière ? (ta demande « path » implique un champ nommé → je pars sur champ nommé.)
- **8.2** Garder le mode **`only` (garder uniquement)** en plus de `hide` dès la 1.0.0 ? (reco : oui,
  peu de coût.)
- **8.3** Partage par lien des règles : inclus (reco) ou non.
- **8.4** Presets réutilisables entre onglets dès maintenant, ou plus tard (reco : plus tard).
