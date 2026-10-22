# Assemblage : fusionner plusieurs sources

[← Sommaire](README.md)

Depuis la 0.8.0, un onglet d'**assemblage** fusionne **plusieurs flux** — des conteneurs Rancher
**et** des topics Kafka, de clusters et de connexions différents — dans **une seule liste triée
dans le temps**, chaque ligne marquée de sa **source** (badge coloré).

## Créer un assemblage

1. Cliquez **⊕ Assemblage** (barre d'onglets, ou l'écran d'accueil quand aucun onglet n'est
   ouvert) : un onglet d'assemblage vide s'ouvre.
2. Tant qu'il est actif, **cliquez un conteneur ou un topic** dans la liste de gauche : il est
   **ajouté comme source** (au lieu d'ouvrir un onglet à part). Répétez (jusqu'à **10** sources).
   - **Ctrl+clic** (ou clic du milieu) ouvre toujours la source dans un onglet séparé.
3. Renommez l'assemblage dans le champ en haut de la vue.

## Lire et filtrer

- Colonne **source** en tête de chaque ligne : badge coloré + nom court (conteneur ou topic).
- **Filtre par source** : décochez une source dans la barre pour masquer ses lignes (sans fermer
  son flux) ; la croix **✕** la retire de l'assemblage.
- Recherche, niveaux, pause, détail, export : comme un onglet de logs normal, appliqués à la vue
  fusionnée (l'export mélange les sources, chaque ligne gardant la sienne).

## Ordre dans le temps

- En **historique**, les lignes sont fusionnées **triées par horodatage**.
- En **suivi** (direct), les lignes de sources différentes arrivent dans le désordre : Ocekour les
  réordonne sur une **fenêtre glissante** réglable (**Réordonner (ms)**, 2 s par défaut).
  **0** = ordre d'arrivée, sans latence. Une ligne très en retard (au-delà de la fenêtre) est
  placée à la suite, jamais perdue ; son horodatage reste visible.
- Les lignes **sans horodatage** prennent l'heure de réception.
- **Ajout d'une source** : son historique est **fusionné par horodatage** avec les lignes déjà
  affichées (pas collé à la fin), tant que la fenêtre de réordonnancement n'est pas à 0.

> Les horloges des serveurs ne sont pas recalées : l'ordre reflète les horodatages d'origine.
> La colonne heure et la source rendent tout écart visible.

## Options

- **Suivi / Historique**, **Lignes**, **Depuis** s'appliquent **à toutes les sources** de
  l'assemblage (un réglage global). Les options par source viendront plus tard.
- Le nombre de lignes gardées en mémoire est le même que pour un onglet simple (réglage
  **Lignes gardées par onglet**), partagé entre toutes les sources.

## Bon à savoir

- L'assemblage est **rouvert au démarrage** (sources, options, filtres, nom).
- Les onglets d'assemblage ne se **partagent pas encore par lien** (prévu plus tard) ; les onglets
  simples, oui.
- Aucune écriture : l'assemblage lit les flux comme l'affichage normal.
