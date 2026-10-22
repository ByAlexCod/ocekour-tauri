# Exporter et partager

[← Sommaire](README.md)

## Exporter des lignes

**Exporter…**, dans la barre d'un onglet de logs, enregistre **les lignes affichées**. Si
un filtre ou des niveaux masqués sont actifs, seules les lignes visibles sont exportées. Le
format dépend de l'extension choisie dans la boîte « Enregistrer sous » :

| Extension                      | Contenu                                                                                                                                                                              |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `.ndjson` / `.jsonl` / `.json` | Une ligne JSON par log : la ligne JSON d'origine, ou `{"time", "message"}` pour du texte. Un message Kafka devient `{"timestamp", "partition", "offset", "key", "headers", "value"}` |
| `.log` / `.txt`                | Les lignes brutes, telles que reçues. Pour Kafka, elles sont préfixées de la date, de `p<partition>@<offset>` et de la clé                                                           |

**Exporter…** n'enregistre que ce qui est **chargé en mémoire** (selon le réglage _Lignes_).

## Télécharger l'historique complet

Depuis la 0.9.0, **Télécharger…** récupère l'**historique complet depuis la création** du/des
pod(s) — pas seulement ce qui est chargé — et propose deux choix :

- **Tout l'historique** : toutes les lignes, sans filtre.
- **Historique filtré** : l'historique complet, avec les **filtres courants** appliqués
  (recherche, niveaux, et sources masquées pour un assemblage).

Le format (NDJSON ou texte) suit l'extension choisie, comme pour l'export. Pour un conteneur
redémarré, les logs de l'**instance précédente** sont inclus quand ils sont encore disponibles.
Pour un **assemblage**, toutes les sources sont récupérées et **fusionnées par horodatage**, chaque
ligne portant sa source (préfixe `[source]` en texte, champ `source` en NDJSON).

> La récupération se fait côté serveur source (API Rancher / Kafka). Un historique très volumineux
> peut être tronqué (une limite de sécurité) et prendre un moment.

Les événements ajoutés par Ocekour (« Conteneur redémarré »…) ne sont pas exportés.

Pour copier **une** ligne, utilisez les boutons **Copier** du panneau de détail.

## Partager une vue avec un collègue

**🔗 Lien**, dans la barre d'un onglet (logs ou explorateur S3), copie un lien de ce type :

```
ocekour://open?d=eyJ2IjoxLCJ0IjoicmFuY2hlciIs…
```

Collez-le dans un message (Rocket.Chat, mail…). Votre collègue le **copie** puis :

- le **colle** dans Ocekour avec **Ctrl+V**, en dehors d'un champ de saisie,
- **ou** clique sur **🔗** en haut de la barre latérale.

Le même onglet s'ouvre chez lui, avec :

| Source  | Contenu du lien                                                                                                                      |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Rancher | Cluster, namespace, pod, conteneur, workload, options (suivi, lignes, depuis, précédent), recherche, niveaux, mode filtrer/surligner |
| Kafka   | Topic, options, recherche, niveaux                                                                                                   |
| S3      | Bucket et dossier courant                                                                                                            |

### Et côté sécurité ?

Le lien ne contient **aucun secret** : ni token, ni mot de passe, ni clé. Il ne contient pas
non plus d'identifiant propre à votre poste. La connexion est retrouvée chez votre collègue
par son **URL** (Rancher, S3) ou par un **broker en commun** (Kafka). Chacun utilise donc ses
propres droits.

### Si la connexion n'existe pas chez votre collègue

- **Aucune connexion ne correspond** : Ocekour propose de la créer, avec l'URL ou les
  brokers déjà remplis. Il ne reste qu'à saisir le secret. Le lien s'ouvre dès
  l'enregistrement.
- **Le cluster n'est pas coché** dans sa connexion Rancher : Ocekour propose de l'ajouter.
- **Le pod n'existe plus**, par exemple parce que le lien date d'avant un redéploiement :
  des boutons proposent d'ouvrir les pods actuels du même workload.

> Le lien n'est pas cliquable dans le navigateur ou la messagerie : c'est volontaire, il
> s'ouvre uniquement par copier-coller dans Ocekour.
