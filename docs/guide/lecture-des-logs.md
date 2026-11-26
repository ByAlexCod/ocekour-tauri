# Lire les logs

[← Sommaire](README.md)

Ce chapitre vaut pour les logs de conteneurs comme pour les messages Kafka.

## Lecture intelligente

Ocekour reconnaît seul les formats courants :

- **JSON sur une ligne** (Logback, pino, Serilog, ECS…) : l'heure, le niveau et le message
  sont extraits de champs comme `@timestamp`, `time`, `level`, `severity`, `message`,
  `msg`…
- **logfmt** (`level=info msg="…"`), texte avec niveau (`INFO`, `WARN`…), couleurs ANSI.
- Horodatages ISO ou epoch (s, ms), affichés **à l'heure locale**.

La liste affiche **heure · niveau · message**. Survolez l'heure pour voir la date complète.

## Rechercher et filtrer

- Saisissez du texte dans **Rechercher (Ctrl+F)**.
  - **Aa** rend la recherche sensible à la casse.
  - **.\*** active les expressions régulières.
- **Filtrer** masque les lignes qui ne correspondent pas. **Surligner** garde toutes les
  lignes, surligne les correspondances et permet de passer de l'une à l'autre avec
  **F3** / **Maj+F3**.
- **Niveaux** (TRACE … FATAL, _autre_) : un clic masque ou affiche un niveau ;
  **Ctrl+clic** n'affiche que ce niveau. Chaque bouton indique le nombre de lignes du niveau.
- **Afficher le contexte** : avec un filtre actif et une ligne sélectionnée, ce bouton
  repasse en mode _Surligner_ avec tous les niveaux et centre la ligne dans le flux complet.
  Vous voyez ainsi ce qui s'est passé juste avant et juste après.

Chaque onglet garde ses propres filtres.

## Colonne à la ligne (texte coupé, JSON indenté)

Le bouton **↵** de l'en-tête d'une colonne l'affiche **sur plusieurs lignes**, directement dans
la liste ; la ligne s'agrandit en conséquence :

- un **texte** (logs non JSON compris) est **coupé à la largeur de la colonne**, de préférence
  entre deux mots ; il se recoupe quand la colonne ou la fenêtre change de largeur ;
- du **JSON** (objet, tableau, ou texte qui en contient) est **indenté**, puis coupé de même.

- Il est disponible sur les colonnes ajoutées au profil et sur la colonne **message**. Pour
  un message Kafka JSON sans champ « message », c'est toute la valeur qui est indentée.
- Le réglage est **enregistré dans le profil** du workload, du topic ou du fichier (`wrap` dans
  `config.yaml`). Il est donc conservé d'un lancement à l'autre.
- Au-delà de 200 lignes, la valeur est tronquée : le détail la montre en entier.

## Copier des lignes

- **Copier tout**, dans la barre de l'onglet, copie dans le presse-papiers **toutes les lignes
  chargées dans l'onglet**, quels que soient les filtres et la recherche.
- **Cliquez** une ligne pour la sélectionner ; **Maj+clic** (ou **Maj+↑ / ↓**) étend la
  sélection à une plage de lignes, surlignée en bleu clair ; **Ctrl+A** sélectionne toutes les
  lignes affichées.
- **Ctrl+C** copie les lignes sélectionnées dans le presse-papiers, telles qu'elles ont été
  reçues. Si du texte est surligné à la souris, Ctrl+C copie ce texte, comme d'habitude.
- Le menu **▾** à côté de « Copier tout » propose aussi : tout l'onglet en **NDJSON**, la
  sélection, ou **les lignes affichées** (filtres compris), en **texte** ou en **NDJSON** (même
  contenu que « Exporter… »).
- Au-delà d'environ 5 Mo de texte, la copie est refusée : utilisez **Exporter…**.

## Afficher, masquer et réordonner les colonnes

Le bouton **Colonnes ▾**, à droite de l'en-tête de la liste, liste toutes les colonnes (heure,
niveau, source d'un assemblage, partition / offset / clé d'un topic, champs ajoutés au profil) :

- **cocher / décocher** une colonne l'affiche ou la masque (une colonne du profil masquée y
  reste : il suffit de la recocher ; ✕ dans son en-tête la retire du profil) ;
- **↑ / ↓** la déplacent vers la gauche ou la droite ; on peut aussi **glisser un en-tête** sur
  un autre ;
- **Réinitialiser** revient à l'ordre par défaut, tout affiché.

Le **message** reste toujours la dernière colonne. L'ordre et les colonnes masquées sont
**enregistrés dans le profil** (`order` et `hidden` dans `config.yaml`) ; dans un assemblage,
ils valent pour l'onglet ouvert.

## Largeur des colonnes

**Glissez** le bord droit de l'en-tête d'une colonne (heure, niveau, source, colonnes du
profil, partition / offset / clé d'un topic) pour changer sa largeur ; **double-cliquez**-le pour
revenir à la largeur par défaut. Le **message** prend toute la place restante.

La largeur est **enregistrée dans le profil** (`widths` dans `config.yaml`), comme les colonnes
affichées. Dans un assemblage, elle vaut pour l'onglet ouvert.

## Le détail d'une ligne

Cliquez sur une ligne pour ouvrir le panneau de détail à droite. La largeur du panneau se
règle avec la séparation.

- **Détail** : l'arbre JSON, lisible. Les chaînes qui contiennent des `\n` (stacktraces,
  requêtes SQL…) s'affichent **sur plusieurs lignes**. Un champ de recherche filtre les
  champs et les valeurs.
- **JSON imbriqué** : affiche en arbre le JSON contenu dans des chaînes.
- **Brut** : la ligne telle que reçue.
- **Copier le JSON** / **Copier le texte** / **Brut** copient la ligne.

Survolez un champ pour faire apparaître deux boutons : **⧉** copie la valeur et **⋯** ouvre
les réglages du champ **pour ce workload** :

| Réglage                                 | Effet                                                    |
| --------------------------------------- | -------------------------------------------------------- |
| Toujours déplier en tête                | Le champ s'affiche en premier dans le détail, déplié (★) |
| Afficher en colonne                     | Le champ devient une colonne de la liste (▥)             |
| Utiliser comme heure / niveau / message | Remplace la détection automatique                        |
| Filtrer sur cette valeur                | Lance une recherche sur cette valeur                     |
| Copier le chemin                        | Copie par ex. `error.stack`                              |

Ces réglages sont enregistrés comme **profil** dans `config.yaml` : ils s'appliquent à tous
les pods du workload, aujourd'hui et après les prochains déploiements. Le bouton
**Profil : …** de la barre d'options indique les profils appliqués et ouvre `config.yaml`
(voir [Configuration](configuration.md#profils)).
