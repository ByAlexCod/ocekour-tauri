# Dépannage et FAQ

[← Sommaire](README.md)

## Messages d'erreur fréquents

| Message                                                                                              | Cause probable                                                               | Que faire                                                                                                                                                             |
| ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| _Certificat de … non reconnu (…)_                                                                    | L'autorité de certification de l'entreprise n'est pas dans Windows           | **Modifier** la connexion → **Ajouter un certificat d'autorité (.cer)…** (demandez le `.cer` à l'équipe infra), ou en dernier recours **Ignorer la vérification SSL** |
| _Serveur … introuvable (DNS)_                                                                        | URL mal saisie, ou hors du réseau / VPN                                      | Vérifiez l'URL et votre connexion réseau                                                                                                                              |
| _Connexion à … impossible (ECONNREFUSED)_                                                            | Mauvais port, service arrêté, pare-feu                                       | Vérifiez le port (Kafka : souvent 9092 ou 9093)                                                                                                                       |
| _Token refusé par Rancher_ / _Authentification Kafka refusée_ / _Clé d'accès ou clé secrète refusée_ | Token Rancher expiré ou révoqué, mauvais mot de passe Kafka, mauvaise clé S3 | **Modifier** la connexion et ressaisir le secret                                                                                                                      |
| _Accès refusé…_ (Rancher, Kafka ACL, S3)                                                             | Vos droits ne couvrent pas ce cluster, ce topic ou ce bucket                 | Demandez les droits (Kafka : `Describe` + `Read` ; S3 : lecture du bucket)                                                                                            |
| _Connexion fermée par le broker : vérifiez le protocole de sécurité_                                 | Protocole incorrect (par ex. `SASL_SSL` au lieu de `SASL_PLAINTEXT`)         | Changez le protocole de sécurité de la connexion                                                                                                                      |
| S3 : _Réponse inattendue : l'URL est-elle bien celle du S3 ?_                                        | L'URL pointe vers une page web ou une console                                | Utilisez l'URL de l'API S3, pas celle de la console                                                                                                                   |
| _Aucun token enregistré pour cette connexion_                                                        | `config.yaml` copié depuis un autre poste (les secrets ne suivent pas)       | **Modifier** la connexion et saisir le secret une fois                                                                                                                |
| _Le chiffrement local est indisponible_                                                              | Dossier de données en lecture seule (disque plein, droits)                   | Ocekour refuse de stocker un secret en clair : libérez de l'espace ou corrigez les droits sur `%APPDATA%\Ocekour`                                                     |

## Clé d'activation

| Message                                           | Que faire                                                                    |
| ------------------------------------------------- | ---------------------------------------------------------------------------- |
| _Ce texte n'est pas une clé Ocekour_              | Copiez la clé en entier : elle commence par `OCEKOUR1-`                      |
| _Clé invalide ou altérée_                         | Un caractère manque ou a changé : recopiez-la depuis le message d'origine    |
| _Clé signée par « … », inconnue de cette version_ | Clé émise pour une version plus récente : mettez Ocekour à jour              |
| _Cette clé a été révoquée_                        | Demandez une nouvelle clé à l'administrateur                                 |
| _Serveur de statistiques injoignable_             | Votre clé impose le suivi : vérifiez le réseau ou le VPN, puis **Réessayer** |

## Questions fréquentes

**Est-ce que lire un topic perturbe les consommateurs ?**
Non. Ocekour ne rejoint aucun groupe de consommateurs et ne commit aucun offset.

**Mes logs n'ont pas le bon niveau ou la bonne heure.**
Ouvrez le détail d'une ligne, cliquez sur **⋯** à côté du bon champ, puis sur
**Utiliser comme niveau** (ou heure, ou message). Le réglage vaut pour tout le workload.

**Je ne retrouve pas mes favoris.**
Ils sont tout en haut de la barre latérale, dans la section **★ Favoris**.

**Un collègue n'arrive pas à ouvrir mon lien.**
Il doit copier le lien **en entier**, puis le coller dans Ocekour (Ctrl+V hors d'un champ)
ou cliquer sur **🔗**. Si la connexion lui manque, Ocekour lui propose de la créer.

**Ocekour s'ouvre puis se ferme aussitôt.**
Depuis la 2.2, une erreur de démarrage s'affiche dans une boîte de message et, si c'est la
fenêtre WebView2 qui échoue, Ocekour se rouvre de lui-même dans le navigateur Edge (mode
application). Le détail est dans `%APPDATA%\Ocekour\logs\main.log`. Pour forcer l'affichage
dans Edge : `Ocekour.exe --navigateur`.

**Ocekour démarre lentement à chaque fois.**
Depuis la 2.0, l'exe ne décompresse plus rien et démarre en moins d'une seconde. Si la
fenêtre reste blanche, vérifiez que **Microsoft Edge WebView2** est installé (Paramètres
Windows → Applications) ou utilisez l'installateur fourni. Le dossier
`%LOCALAPPDATA%\Ocekour\app\` des versions 1.x peut être supprimé sans risque ; vos réglages
sont dans `%APPDATA%\Ocekour`.

**Après un redémarrage / changement de session, Ocekour redemande la clé et les jetons.**
Depuis la 0.5.0, les secrets sont chiffrés par une **clé locale** (`secret.key`), plus par
Windows : un changement de session ne les casse plus. Si `secret.key` est supprimé ou que le
fichier `secrets.json` est copié sans lui, les secrets deviennent illisibles — ce n'est pas
une perte de données côté Ocekour : ressaisissez la clé d'activation, puis vos jetons dans
Paramètres → Comptes (le journal liste ce qui est concerné). Le jeton du serveur MCP est
régénéré tout seul : reconfigurez vos clients MCP avec le nouveau jeton (Paramètres → Serveur
MCP). En passant d'une version antérieure à la 0.5.0, les secrets encore lisibles sont migrés
automatiquement.

**Je ne reçois pas de notifications Rocket.Chat.**
Vérifiez, dans Paramètres → Notifications : la case principale cochée, un jeton pour chaque
source surveillée ET pour Rocket.Chat (Paramètres → Comptes), et le bouton « Envoyer un
message d'essai ». Les notifications ne partent que pendant qu'Ocekour tourne (fenêtre
ouverte, ou lancé avec `--notify`).

**Ocekour consomme beaucoup de mémoire.**
Réduisez **Lignes gardées par onglet** dans les Paramètres et fermez les onglets inutiles.

## Signaler un problème

Joignez le **journal** `%APPDATA%\Ocekour\logs\main.log` (**Paramètres → Afficher le journal**), la version (nom de l'exe) et les
étapes pour reproduire. Le journal ne contient pas vos secrets.
