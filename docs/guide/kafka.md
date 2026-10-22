# Kafka : messages des topics

[← Sommaire](README.md)

Ocekour lit les topics **en lecture seule**, **sans groupe de consommateurs et sans commit**.
Chaque partition est lue directement, ce qui ne décale aucun offset et n'a **aucun impact**
sur les applications qui consomment ces topics. Ocekour ne peut pas écrire dans Kafka.

## Créer la connexion

**+** → **Kafka**, puis :

| Champ                              | Valeur                                                                                                                            |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| **Brokers**                        | `bootstrap.servers`, séparés par des virgules : `kafka1.intra:9092, kafka2.intra:9092`                                            |
| **Sécurité**                       | `SASL_PLAINTEXT` + `SCRAM-SHA-512` par défaut. Aussi `SASL_SSL`, `PLAINTEXT`, `SCRAM-SHA-256`, `PLAIN`                            |
| **Utilisateur** / **Mot de passe** | `sasl.username` / `sasl.password`. Le mot de passe est chiffré localement (clé du poste)                                          |
| **Certificat**                     | Pour `SASL_SSL` seulement : même principe que pour Rancher (autorités Windows, fichier `.cer`, ou vérification ignorée)           |
| **Schema Registry**                | Facultatif, pour décoder l'Avro : `http://schema-registry.intra:8081`. Sans authentification ; en HTTPS, même choix de certificat |

**Tester la connexion** affiche par exemple « 3 broker(s), 97 topic(s), Schema Registry OK
(42 sujet(s)) ».

Pour plusieurs clusters Kafka, créez une connexion par cluster. Ocekour a été testé avec
Kafka 3.0 et 4.3.

Droits nécessaires : `Describe` et `Read` sur les topics. Aucun droit sur les groupes n'est
nécessaire.

## Parcourir

Les topics sont listés sous la connexion, sans les topics internes `__…`, avec leur nombre
de partitions. Au-delà de 20 topics, ils sont **regroupés par préfixe** (`commandes.…`,
`paiements.…`). Le champ **Filtrer** et les **favoris** fonctionnent comme pour Rancher.

## Lire un topic

Cliquez sur un topic. Les options de la barre changent de sens :

| Option                              | Effet sur un topic                                                                   |
| ----------------------------------- | ------------------------------------------------------------------------------------ |
| **Lignes : N**                      | Les **N derniers messages du topic**, toutes partitions confondues et triés par date |
| **Lignes : Tout (depuis le début)** | Tout le topic, depuis le plus ancien message conservé                                |
| **Depuis**                          | Les messages postérieurs à la date choisie (5 min, 1 h, 24 h)                        |
| **Suivi**                           | Continue ensuite en direct. **Historique** s'arrête à la fin actuelle du topic       |

La liste ajoute les colonnes **partition**, **offset** et **clé**. La recherche porte aussi
sur la **clé** et les **headers**.

Le **détail** d'un message affiche :

- la partition, l'offset et le **format** détecté ;
- la **clé** et les **headers** ;
- la valeur, lisible comme une ligne de log (arbre JSON, champs multilignes, profils…).

## Formats reconnus

Le format de la clé et de la valeur est détecté automatiquement :

| Format      | Détection                                                                                                                                     |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| **Avro**    | Format Confluent (octet magique + id de schéma) : le schéma est demandé au Schema Registry, puis mis en cache. La valeur est affichée en JSON |
| **JSON**    | Texte JSON valide                                                                                                                             |
| **texte**   | Texte UTF-8                                                                                                                                   |
| **binaire** | Tout le reste, affiché en hexadécimal                                                                                                         |

Si un message est au format Avro mais qu'aucun registre n'est configuré, ou que le schéma
est introuvable, un **avertissement** s'affiche dans le détail et la valeur est montrée
brute.

## Profils

Les réglages « en un clic » du détail (colonnes, champs mis en avant…) s'appliquent au
topic. Dans `config.yaml`, ils sont rangés sous la clé `kafka/<nom de la connexion>/<topic>`
(voir [Configuration](configuration.md#profils)).
