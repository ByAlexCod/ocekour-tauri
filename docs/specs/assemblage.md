# Spec — Assemblage : vue fusionnée multi-sources (0.8.0)

> Statut : **implémentée en 0.8.0** · Guide utilisateur : [assemblage.md](../guide/assemblage.md)
> · S'appuie sur les onglets de logs existants (Rancher, Kafka)
>
> Écarts avec la proposition : options de flux **globales** (par-source reporté, §7) ; profils
> **neutres** pour l'assemblage (base heure · source · niveau · message ; profils par source
> reportés) ; **partage par lien reporté** (les onglets simples restent partageables). Le reste
> est conforme : fusion multi-sources, fenêtre de réordonnancement (2 s, 0 = arrivée), colonne et
> filtre par source, persistance de session, max 10 sources.

## 1. Objectif

Aujourd'hui, **un onglet affiche une seule cible** : un conteneur Rancher **ou** un topic Kafka.
Pour corréler un incident entre plusieurs services, l'utilisateur doit basculer d'un onglet à
l'autre. L'**assemblage** introduit un onglet qui **fusionne plusieurs cibles de flux** — des pods
**et** des topics, éventuellement de clusters et de connexions différents — dans **une seule liste
triée dans le temps**, chaque ligne étiquetée par sa source.

Principes :

- **Réutilise le socle existant** : chaque membre est un flux ouvert par `streams.open` ; aucun
  nouveau protocole, aucun accès réseau nouveau.
- **Un seul tampon** de lignes pour la vue fusionnée, avec une **colonne « source »** et un
  **filtre par source**.
- Recherche, niveaux, pause, export, profils : comme un onglet de logs normal, appliqués à la
  vue fusionnée.
- **Lecture seule** : l'assemblage n'écrit rien, ne touche ni aux notifications ni au contexte IA.

Hors périmètre 0.8.x : les buckets S3 (ce ne sont pas des flux) ; la corrélation automatique
(trace-id) entre sources ; l'alignement d'horloges entre serveurs ; la fusion d'assemblages.

## 2. Modèle

Nouveau type d'onglet, à côté de `container`, `topic`, `bucket` :

```ts
export interface AssemblyTarget {
  kind: 'assembly';
  /** Identifiant stable de l'assemblage (clé d'onglet, lien de partage). */
  id: string;
  /** Nom affiché, par défaut « Assemblage (N) », renommable. */
  name?: string;
  members: AssemblyMember[];
}

export interface AssemblyMember {
  /** Id stable du membre dans l'assemblage (couleur, filtre, étiquette). */
  id: string;
  /** La cible de flux réutilisée telle quelle. */
  target: ContainerTarget | TopicTarget;
  /** Libellé court (conteneur / topic) et couleur du badge. */
  label: string;
  color: string;
}
```

- `targetKey` d'un assemblage = `assembly/<id>` (un assemblage est unique par son id, pas par ses
  membres : on peut avoir deux assemblages différents sur des sous-ensembles qui se recoupent).
- **Au plus `MAX_MEMBERS = 10`** membres par assemblage (bornage réseau/CPU). _(reco)_
- Couleurs : palette fixe de 10 teintes distinctes, assignées dans l'ordre d'ajout.

## 3. Backend : N flux, lignes étiquetées

- À l'ouverture, le front ouvre **un flux par membre** via `streams.open` (inchangé), avec les
  **mêmes options** pour tous (§7). Chaque flux garde son `streamId`.
- Les événements (`lines`, `status`, `notice`) arrivent **par membre** : le front sait de quel
  `streamId` vient chaque lot et **étiquette chaque ligne** du `memberId` correspondant.
- `streams.close` est appelé pour chaque membre à la fermeture de l'onglet (déjà géré par flux).
- Aucune modification du `StreamManager` : l'assemblage est une **composition côté front**.

## 4. Fusion et ordre (point délicat)

Chaque ligne porte un horodatage (`ts`) : log Rancher = heure du log ; message Kafka = heure du
message. **Les lignes sans horodatage** (brut non daté) prennent **l'heure de réception**.

- **Historique** (chargement initial, `tailLines`) : chaque membre apporte son lot ; on **fusionne
  par tri temporel** une fois les lots initiaux reçus. Ordre strict et correct.
- **Direct** (suivi) : les lignes de sources différentes arrivent dans le désordre. On applique un
  **réordonnancement sur une fenêtre glissante** : les lignes reçues sont gardées `REORDER_MS` puis
  insérées triées par `ts` dans le tampon affiché. _(reco : `REORDER_MS = 2000`, réglable ;
  **0 = ordre d'arrivée**, sans latence.)_ **← à confirmer (question 3).**

> La fenêtre de réordonnancement borne le désordre inter-sources sans exiger d'horloges
> parfaitement alignées ; au-delà de la fenêtre, une ligne très en retard est insérée à la fin
> (jamais perdue), avec son horodatage visible.

Le tampon unique réutilise `LogBuffer` (capacité = `bufferSize`, comme un onglet normal). Au-delà,
les plus anciennes lignes (tous membres confondus) sont retirées.

## 5. Affichage

- Colonne **« source »** en tête de chaque ligne : **badge coloré** + libellé court (conteneur ou
  topic). Infobulle = chemin complet (cluster/namespace/pod, ou connexion/topic).
- **Filtre par source** : une barre de cases à cocher (une par membre, avec sa couleur) pour
  masquer/afficher chaque source sans fermer le flux. Cumulable avec la recherche et les niveaux.
- **Profils** : chaque ligne est analysée avec le profil de **sa** source (`profileKey` du membre).
  Colonnes affichées = base commune (**heure · source · niveau · message**) + colonnes de profil
  présentes ; une colonne de profil propre à une source reste vide pour les autres.
- Recherche, niveaux, pause, « coller en bas », détail d'une ligne, export : identiques à un
  onglet de logs, portant sur la vue fusionnée (export = lignes mélangées, avec la source).

## 6. Création et édition

1. **Depuis le parcours** : cases à cocher dans l'arbre (plusieurs conteneurs / topics), puis
   **« Ouvrir en assemblage »**. _(reco)_
2. **Depuis un onglet assemblage** : bouton **« + Ajouter une source »** (ouvre le parcours), et
   **retrait** d'un membre (croix sur son badge, ferme son flux).
3. **Renommer** l'onglet (double-clic sur le titre / menu ⋯).

Un membre retiré ferme son flux ; un membre ajouté ouvre le sien avec les options courantes (§7).

## 7. Options de flux

_(reco — **à confirmer, question 8**)_ : **un réglage global** pour tout l'assemblage (suivi
oui/non, nombre de lignes initiales), appliqué à chaque membre à l'ouverture et à l'ajout. Les
options par source (un topic depuis le début pendant qu'un pod suit) sont **hors périmètre 0.8.0**
et pourront venir plus tard.

- Kafka et Rancher partagent déjà le même `StreamOptions` (`follow`, `tailLines`, `sinceSeconds`).
- Le changement d'options **rouvre** tous les flux membres (comme un onglet simple aujourd'hui).

## 8. Persistance et partage

- **Session** : un assemblage est un `TabState` comme les autres ; il est **rouvert au démarrage**
  (ses membres, ses options, son nom, ses filtres de source).
- **Lien de partage** `ocekour://open?d=…` : encode `kind: assembly` + la liste des membres (chemins
  de connexion, **sans secret**). À l'ouverture chez un collègue, chaque membre est résolu vers sa
  connexion locale (comme aujourd'hui) ; les membres dont la connexion est absente sont **listés**
  et ignorés (l'assemblage s'ouvre avec les membres résolubles).
- Migration : les anciens `TabState` (sans `kind` assembly) restent inchangés.

## 9. Découpage

Tout en **0.8.0** :

1. Modèle `AssemblyTarget` / `AssemblyMember`, clé d'onglet, couleurs, titres.
2. Session d'assemblage côté front : N flux, étiquetage par membre, tampon fusionné, fenêtre de
   réordonnancement.
3. Affichage : colonne source, filtre par source, profils par source, export.
4. Création (multi-sélection du parcours + « Ouvrir en assemblage »), ajout/retrait, renommage.
5. Persistance de session + lien de partage (encodage/décodage multi-membres).
6. Tests (fusion/ordre, filtre par source, partage), guide utilisateur, version, CI.

## 10. Risques

- **Débit cumulé** : N flux à fort débit → le tampon se remplit vite. Mitigation : `MAX_MEMBERS`,
  capacité partagée `bufferSize`, indicateur de débit par source.
- **Latence du direct** vs exactitude de l'ordre : réglée par `REORDER_MS` (compromis assumé,
  réglable, 0 possible).
- **Horloges non alignées** entre serveurs : l'ordre reflète les horodatages des serveurs ; on ne
  recale pas les horloges (hors périmètre). La colonne heure + source rend l'écart visible.
- **Profils hétérogènes** : colonnes potentiellement creuses ; on garde une base commune lisible.
