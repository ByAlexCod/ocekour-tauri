# Spec — Outils MCP « Bases de données » (lecture seule) (1.3.0)

> Statut : **à implémenter en 1.3.0** · Guide utilisateur : [bdd.md](../guide/bdd.md)

## 1. Objectif

Donner aux agents (via le serveur MCP) un accès **en lecture seule** à des bases de données
d'entreprise pour **explorer le schéma** et **analyser les données** (SELECT), **jamais écrire**.

Moteurs de base : **PostgreSQL, SQL Server, Oracle**. Objectif : **le plus large panel de
versions**, **tous les drivers inclus** dans l'exe (rien à installer côté poste).

## 2. Drivers (100 % JavaScript, aucun binaire natif à recompiler)

- **PostgreSQL** : `pg` (pur JS) — PostgreSQL 8 → 17+ (et compatibles : Redshift, etc.).
- **SQL Server** : `mssql` / `tedious` (pur JS) — SQL Server 2005 → 2022 et Azure SQL.
- **Oracle** : `oracledb` en **mode « thin »** (pur JS, **sans** Oracle Instant Client) — Oracle
  Database 12.1 → 23ai.

`pg` et `mssql` sont **bundlés** (esbuild). `oracledb` charge un `.node` selon la plateforme
(mode thick, non utilisé) → **externe** au bundle, chargé paresseusement et empaqueté par
electron-builder (`asarUnpack`). Le mode thin ne touche jamais ce binaire : pur JS.

## 3. Lecture seule — défense en profondeur

1. **Garde SQL** (`sql-guard.ts`, pure et testée) : une **seule** instruction ; début dans
   `SELECT`/`WITH`/`EXPLAIN`/`SHOW`/`DESC` ; rejet de tout mot-clé d'écriture / DDL / transaction /
   procédure (`INSERT`, `UPDATE`, `DELETE`, `MERGE`, `CREATE`, `DROP`, `ALTER`, `TRUNCATE`,
   `GRANT`, `CALL`, `SET`, `INTO`…). Commentaires et chaînes neutralisés avant analyse.
2. **Transaction read-only** côté moteur : PostgreSQL `BEGIN TRANSACTION READ ONLY` ; Oracle
   `SET TRANSACTION READ ONLY` ; SQL Server `readOnlyIntent`.
3. **Bornes** : plafond de lignes (défaut 200, max 10 000) et **timeout** (30 s).
4. Recommandation : utiliser en plus un **compte DB en lecture seule**.

## 4. Configuration

Section `databases` de `config.yaml` (jamais de mot de passe dedans) :

```yaml
databases:
  - id: db-xxxx
    engine: postgres | sqlserver | oracle
    name: Ma base
    host: db.exemple.fr
    port: 5432            # défaut : 5432 / 1433 / 1521
    database: ventes      # Oracle : service name (ou SID si oracleSid: true)
    user: lecteur
    ssl: false
    tls: { insecure: false }   # ou caFile: …
```

Le **mot de passe** est chiffré dans `secrets.json` (clé = `id`), saisi via **Paramètres →
Bases de données** (écran type connexion : moteur, hôte, port, base, utilisateur, mot de passe,
TLS, **Tester**). Découplé de l'arbre des sources de logs.

## 5. Outils MCP (`bdd_*`, tous en lecture seule)

| Outil | Rôle |
| ----- | ---- |
| `bdd_connexions` | Liste les bases configurées |
| `bdd_tables(connexion)` | Schémas, tables et vues (hors objets système) |
| `bdd_colonnes(connexion, table, schema?)` | Colonnes (nom, type, nullable) |
| `bdd_apercu(connexion, table, schema?, limite?)` | Premières lignes (SELECT * borné) |
| `bdd_requete(connexion, sql, limite?)` | Un SELECT/WITH/EXPLAIN borné |
| `bdd_expliquer(connexion, sql)` | Plan d'exécution (PostgreSQL) |

Visibles seulement si ≥ 1 base configurée **et** licence active.

## 6. Découpage (1.3.0)

1. Schéma `databases` + types (`shared/config.ts`, `shared/api.ts`).
2. `electron/bdd/` : `sql-guard.ts` (+ tests), `types.ts`, `drivers.ts` (pg/mssql/oracle),
   `manager.ts`, `tools.ts`.
3. IPC `databases:*` + preload + `McpContext.databases` + instructions du serveur.
4. UI Paramètres → Bases de données.
5. Empaquetage (externe oracledb, asarUnpack), guide, CHANGELOG, version.

## 7. Hors périmètre (plus tard)

- MySQL/MariaDB, SQLite, Db2 (ajout facile : un driver de plus).
- Écriture (jamais prévue), appels de procédures, export de gros résultats.
- Oracle en mode thick / wallet TLS avancé, LDAP (nécessiterait Instant Client).
