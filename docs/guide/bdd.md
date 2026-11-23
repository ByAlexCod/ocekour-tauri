# Bases de données : analyse en lecture seule pour vos agents

[← Sommaire](README.md)

Depuis la 1.3.0, Ocekour peut donner à vos agents (via le serveur MCP) un accès **en lecture
seule** à vos bases de données : explorer le schéma, lire des tables, lancer des `SELECT`.
**Aucune écriture n'est possible.**

Moteurs pris en charge, **tous les drivers inclus** dans l'exe (rien à installer) :

| Moteur | Versions | Détail |
| ------ | -------- | ------ |
| **PostgreSQL** | 8 → 17+ | driver `pg` |
| **SQL Server** | 2005 → 2022, Azure SQL | driver `tedious` |
| **Oracle** | 12.1 → 23ai | mode « thin », **sans** Oracle Instant Client |

## Ajouter une base

**Paramètres → Bases de données** : choisissez le moteur, renseignez hôte, port, base
(pour Oracle : *service name*, ou cochez **SID**), utilisateur et mot de passe, puis **Tester**
et **Ajouter**.

- Le mot de passe est **chiffré sur ce poste** (`secrets.json`) et **jamais** écrit dans
  `config.yaml` ni transmis aux agents.
- **TLS/SSL** : cochez « Connexion TLS/SSL » ; au besoin, « ne pas vérifier le certificat » ou un
  certificat d'autorité (CA d'entreprise), comme pour les autres connexions.

> Conseil : créez côté serveur un **compte en lecture seule** dédié. Ocekour empêche déjà toute
> écriture, mais un compte en lecture seule est une sécurité supplémentaire.

## Les outils exposés (lecture seule)

| Outil | Rôle |
| ----- | ---- |
| `bdd_connexions` | Liste les bases configurées |
| `bdd_tables` | Schémas, tables et vues |
| `bdd_colonnes` | Colonnes d'une table (nom, type, nullable) |
| `bdd_apercu` | Premières lignes d'une table |
| `bdd_requete` | Une requête `SELECT` (résultat borné) |
| `bdd_expliquer` | Plan d'exécution (PostgreSQL) |

Les outils n'apparaissent **que** si au moins une base est configurée et la licence active.

## Comment la lecture seule est garantie

1. **Une seule instruction** par appel, commençant par `SELECT` / `WITH` / `EXPLAIN` / `SHOW`.
2. Tout mot-clé d'écriture ou de structure (`INSERT`, `UPDATE`, `DELETE`, `CREATE`, `DROP`,
   `ALTER`, `TRUNCATE`, `GRANT`, `CALL`…) est **refusé**.
3. La requête s'exécute dans une **transaction en lecture seule** côté serveur.
4. Le résultat est **borné** (lignes et temps d'exécution).

## Oracle : redirections du listener

Le listener Oracle peut renvoyer le client vers une autre adresse : dispatcher de serveur
partagé, nœud d'un RAC (listener SCAN), Connection Manager, ou processus dédié sous Windows.
Depuis la 2.3.0, Ocekour **suit la redirection** (jusqu'à 4 de suite) avec la chaîne de
connexion fournie par le listener ; avant, la connexion échouait sur *connection redirected to:
redirect not implemented*.

L'adresse de redirection doit être joignable depuis le poste (nom d'hôte résolu, port ouvert).
Sinon, l'erreur indique l'adresse visée (`hôte:port`) : c'est elle qu'il faut ouvrir, ou
demander au DBA de déclarer dans le listener une adresse joignable (`LOCAL_LISTENER`, SCAN).
