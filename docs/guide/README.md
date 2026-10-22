# Ocekour — Guide utilisateur

> **Version 2.2.0** · Documentation versionnée avec le code (`docs/guide/` du dépôt).
> Historique des versions : [CHANGELOG.md](CHANGELOG.md).

Ocekour est une visionneuse de logs **locale** pour Windows 10/11. C'est un seul exécutable
portable, sans installation. Elle lit :

| Source              | Ce que vous voyez                                                                             |
| ------------------- | --------------------------------------------------------------------------------------------- |
| **Rancher** (2.5+)  | Les logs des conteneurs Kubernetes, en direct ou en historique                                |
| **Kafka**           | Les messages des topics (JSON, texte, Avro), en lecture seule et sans groupe de consommateurs |
| **S3** d'entreprise | Un explorateur de buckets : aperçu, téléchargement, envoi, renommage, suppression             |
| **Fichiers**        | Les fichiers de logs du poste (fichier ou motif `app-*.log`), comme un pod                    |
| **Sonde**           | Les logs d'une machine distante (VM, serveur Windows/Linux), exposés par `ocekour-headless`   |

Ocekour est aussi le **hub MCP de vos agents** (Claude Code, Qwen Code, Mistral Vibe…) : un
seul serveur MCP local leur donne accès à ces sources et à **GitLab, Jira, Confluence et
Rocket.Chat**, avec vos propres jetons.

Tout reste sur votre poste. Il n'y a ni serveur ni base de données, et aucune donnée n'est
envoyée ailleurs que vers vos sources. Les secrets (token, mots de passe, clés) sont
chiffrés localement sur le poste (clé dans un fichier à côté des données) et ne sont jamais envoyés ailleurs que vers vos sources.

## Sommaire

1. [Installation et premier lancement](installation.md)
2. [Prise en main : interface, onglets, raccourcis](prise-en-main.md)
3. [Rancher : logs des conteneurs](rancher.md)
4. [Lire les logs : recherche, niveaux, détail, profils](lecture-des-logs.md)
5. [Kafka : messages des topics](kafka.md)
6. [S3 : explorateur de buckets](s3.md)
7. [Exporter et partager](partage-export.md)
8. [Configuration (`config.yaml`)](configuration.md)
9. [Serveur MCP : Ocekour pour les agents](mcp.md)
10. [Hub MCP : GitLab, Jira, Confluence, Rocket.Chat](hub-mcp.md)
11. [Notifications vers Rocket.Chat](notifications.md)
12. [Contexte IA : mémoire pour vos agents](contexte-ia.md)
13. [Assemblage : fusionner plusieurs sources](assemblage.md)
14. [Filtres de logs](filtres.md)
15. [Web & navigateur : outils MCP pour vos agents](web.md)
16. [Bases de données : analyse en lecture seule](bdd.md)
17. [Fichiers locaux : lire les logs du poste](fichiers-locaux.md)
18. [Sonde : les logs d'une machine distante](sonde.md)
17. [Dépannage et FAQ](depannage.md)

## En 30 secondes

1. Double-cliquez sur `Ocekour-1.3.1.exe` et collez votre **clé d'activation**.
2. Cliquez sur **+** en haut à gauche, choisissez **Rancher**, **Kafka** ou **S3**, remplissez les champs et cliquez sur **Tester**, puis **Enregistrer**.
3. Dépliez l'arbre et cliquez sur un conteneur ou un topic pour voir ses logs, ou sur un bucket pour l'explorer.
4. Pour montrer ce que vous voyez à un collègue, cliquez sur **🔗 Lien** et envoyez-lui le lien. Il le colle dans son Ocekour avec **Ctrl+V**.
