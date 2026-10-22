import { z } from 'zod';

// Schéma de %APPDATA%\Ocekour\config.yaml — source de vérité unique (main et front).
// Le front n'importe que les types (`import type`) : zod n'est pas embarqué côté UI.

export const ThemeSchema = z.enum(['system', 'dark', 'light']);

const TlsSchema = z.object({
  /** Ne vérifie pas le certificat du serveur. */
  insecure: z.boolean().default(false),
  /** Certificat d'autorité (.cer / .crt / .pem, DER ou PEM). */
  caFile: z.string().optional(),
});

const RancherConnectionSchema = z.object({
  id: z.string().min(1),
  type: z.literal('rancher'),
  name: z.string().min(1),
  url: z.url(),
  tls: TlsSchema.default({ insecure: false }),
  /** Clusters Rancher cochés par l'utilisateur. */
  clusters: z.array(z.object({ id: z.string().min(1), name: z.string() })).default([]),
});

const KafkaConnectionSchema = z.object({
  id: z.string().min(1),
  type: z.literal('kafka'),
  name: z.string().min(1),
  /** `hôte:port`, au moins un. */
  brokers: z.array(z.string().min(1)).min(1),
  security: z
    .object({
      protocol: z.enum(['SASL_PLAINTEXT', 'SASL_SSL', 'PLAINTEXT']).default('SASL_PLAINTEXT'),
      mechanism: z.enum(['SCRAM-SHA-512', 'SCRAM-SHA-256', 'PLAIN']).default('SCRAM-SHA-512'),
      username: z.string().optional(),
    })
    .default({ protocol: 'SASL_PLAINTEXT', mechanism: 'SCRAM-SHA-512' }),
  /** Certificat du broker (SASL_SSL uniquement). */
  tls: TlsSchema.default({ insecure: false }),
  /** Schema Registry (Avro), sans authentification. */
  registry: z.object({ url: z.url(), tls: TlsSchema.default({ insecure: false }) }).optional(),
});

const S3ConnectionSchema = z.object({
  id: z.string().min(1),
  type: z.literal('s3'),
  name: z.string().min(1),
  url: z.url(),
  region: z.string().optional(),
  /** `https://s3.intra/bucket/clé` (le cas des S3 d'entreprise). */
  pathStyle: z.boolean().default(true),
  accessKeyId: z.string().min(1),
  tls: TlsSchema.default({ insecure: false }),
  /** Buckets saisis à la main (si le compte ne peut pas les lister). */
  buckets: z.array(z.string().min(1)).default([]),
});

export const ConnectionSchema = z.discriminatedUnion('type', [
  RancherConnectionSchema,
  KafkaConnectionSchema,
  S3ConnectionSchema,
]);

/** Services accessibles aux agents via le MCP (un par type ; jeton dans secrets.json). */
export const SERVICE_IDS = ['gitlab', 'jira', 'confluence', 'rocketchat'] as const;
export const ServiceIdSchema = z.enum(SERVICE_IDS);

export const ServiceSchema = z.object({
  url: z.url(),
  tls: TlsSchema.default({ insecure: false }),
  /** Outils visibles par l'agent (si un jeton est enregistré). */
  enabled: z.boolean().default(true),
  /** Outils d'écriture visibles par l'agent. */
  write: z.boolean().default(false),
});

export const ServicesSchema = z.object({
  gitlab: ServiceSchema.optional(),
  jira: ServiceSchema.optional(),
  confluence: ServiceSchema.optional(),
  rocketchat: ServiceSchema.optional(),
});

/** Sources dont Ocekour surveille les notifications (celles qui ont un compte). */
export const NOTIFICATION_SOURCES = ['gitlab', 'jira', 'confluence'] as const;
export const NotificationSourceSchema = z.enum(NOTIFICATION_SOURCES);

const NotificationSourceConfig = z.object({
  enabled: z.boolean().default(true),
  /** Types d'événements retenus ; vide = les types par défaut de la source. */
  events: z.array(z.string()).default([]),
});
const sourceDefault = () => NotificationSourceConfig.parse({});

const NotificationSourcesSchema = z.object({
  gitlab: NotificationSourceConfig.default(sourceDefault),
  jira: NotificationSourceConfig.default(sourceDefault),
  confluence: NotificationSourceConfig.default(sourceDefault),
});

/**
 * Notifications : Ocekour interroge GitLab, Jira et Confluence et envoie, pour ce qui
 * concerne directement l'utilisateur, un message groupé à soi-même dans Rocket.Chat.
 */
export const NotificationsSchema = z.object({
  enabled: z.boolean().default(false),
  /** Intervalle entre deux scrutations (secondes). */
  intervalSeconds: z.int().min(30).max(3_600).default(60),
  /** Destinataire : `self` (soi-même), `#canal` ou `@personne`. */
  target: z.string().default('self'),
  /** Afficher aussi les nouveautés en notifications système (Windows). */
  desktop: z.boolean().default(true),
  sources: NotificationSourcesSchema.default(() => NotificationSourcesSchema.parse({})),
});

/** Sources ingérées par la mémoire de contexte IA. */
export const CONTEXT_SOURCES = ['rocketchat', 'gitlab', 'jira', 'confluence'] as const;
export const ContextSourceSchema = z.enum(CONTEXT_SOURCES);

const ContextSourceConfig = z.object({ enabled: z.boolean().default(true) });
const ctxSourceDefault = () => ContextSourceConfig.parse({});

const ContextSourcesSchema = z.object({
  rocketchat: ContextSourceConfig.default(ctxSourceDefault),
  gitlab: ContextSourceConfig.default(ctxSourceDefault),
  jira: ContextSourceConfig.default(ctxSourceDefault),
  confluence: ContextSourceConfig.default(ctxSourceDefault),
});

/**
 * Contexte IA : chaque message Rocket.Chat reçu et chaque nouveauté GitLab / Jira / Confluence
 * alimentent une mémoire de contexte multi-fichiers, exposée aux agents locaux par le MCP.
 * Désactivée par défaut ; à la première activation, part de « maintenant » (pas de rattrapage).
 */
export const ContextSchema = z.object({
  enabled: z.boolean().default(false),
  /** Intervalle entre deux scrutations (secondes). */
  intervalSeconds: z.int().min(30).max(3_600).default(60),
  /** LLM de curation : lu dans la config de Qwen / Vibe (`off` = stockage brut). */
  llm: z.enum(['auto', 'qwen', 'vibe', 'off']).default('auto'),
  /** TLS de l'endpoint LLM (https auto-signé / CA d'entreprise), comme les autres services. */
  tls: TlsSchema.default({ insecure: false }),
  sources: ContextSourcesSchema.default(() => ContextSourcesSchema.parse({})),
});

/**
 * Accès web & contrôle de navigateur, exposés aux agents par le MCP (outils web_*).
 * Pilote le navigateur du poste (Edge / Chrome) via Playwright ; aucun binaire téléchargé.
 * Désactivé par défaut ; à la première activation, lancé paresseusement.
 */
export const WebSchema = z.object({
  enabled: z.boolean().default(false),
  /** Navigateur piloté : canal Playwright du navigateur installé sur le poste. */
  browser: z.enum(['msedge', 'chrome', 'chromium']).default('msedge'),
  /** Fenêtre invisible (true) ou visible (false, défaut : l'utilisateur voit l'agent agir). */
  headless: z.boolean().default(false),
  /** Délai de navigation / action (ms). */
  timeoutMs: z.int().min(1_000).max(120_000).default(30_000),
  /** TLS de web_recuperer et du navigateur (https auto-signé / CA d'entreprise). */
  tls: TlsSchema.default({ insecure: false }),
});

/**
 * Base de données consultable par les agents (lecture seule) via le MCP. Le mot de passe n'est
 * jamais dans config.yaml : il est chiffré dans secrets.json (clé = id de la base).
 */
export const DATABASE_ENGINES = ['postgres', 'sqlserver', 'oracle'] as const;
export const DatabaseEngineSchema = z.enum(DATABASE_ENGINES);

export const DatabaseSchema = z.object({
  id: z.string().min(1),
  engine: DatabaseEngineSchema,
  name: z.string().min(1),
  host: z.string().min(1),
  /** Défaut selon le moteur : PostgreSQL 5432, SQL Server 1433, Oracle 1521. */
  port: z.int().min(1).max(65_535).optional(),
  /** PostgreSQL / SQL Server : nom de la base. Oracle : service name (ou SID si `oracleSid`). */
  database: z.string().optional(),
  user: z.string().min(1),
  /** Chiffrer la connexion (TLS/SSL). */
  ssl: z.boolean().default(false),
  /** Oracle : interpréter `database` comme SID plutôt que service name. */
  oracleSid: z.boolean().default(false),
  /** Vérification du certificat quand `ssl` est actif (insecure / CA d'entreprise). */
  tls: TlsSchema.default({ insecure: false }),
});

export const ProfileSchema = z.object({
  /** `cluster/namespace/workload`, joker `*`. Le premier profil qui correspond gagne. */
  match: z.string().min(1),
  fields: z
    .object({
      time: z.string().optional(),
      level: z.string().optional(),
      message: z.string().optional(),
    })
    .default({}),
  columns: z.array(z.string()).default([]),
  expand: z.array(z.string()).default([]),
  /**
   * Colonnes dont la valeur JSON s'affiche indentée, sur plusieurs lignes, dans la liste
   * (chemin de champ, ou `@message` pour la colonne message).
   */
  wrap: z.array(z.string()).default([]),
  parseNested: z.boolean().optional(),
});

/** Règle d'un filtre enregistré (modèle de l'encart Filtres, sans l'`id` propre à l'onglet). */
export const StoredFilterRuleSchema = z.object({
  enabled: z.boolean().default(true),
  /** `@line` (ligne brute), `@message` (message analysé), ou un chemin de champ JSON. */
  field: z.string().min(1).default('path'),
  mode: z.enum(['exact', 'contains', 'regex']).default('contains'),
  value: z.string().default(''),
  caseSensitive: z.boolean().default(false),
  action: z.enum(['hide', 'only']).default('hide'),
});

/** Jeu de règles nommé, enregistré et réutilisable entre onglets (tous les pods). */
export const FilterPresetSchema = z.object({
  name: z.string().min(1),
  rules: z.array(StoredFilterRuleSchema).default([]),
});

export const PreferencesSchema = z.object({
  theme: ThemeSchema.default('system'),
  /** Lignes gardées en mémoire par onglet. */
  bufferSize: z.int().min(1_000).max(1_000_000).default(100_000),
  /** Lignes chargées à l'ouverture d'un flux. */
  defaultTailLines: z.int().min(0).max(1_000_000).default(1_000),
  restoreTabs: z.boolean().default(true),
  /** Dossier partagé où sont déposées les nouvelles versions (optionnel). */
  updateShare: z.string().optional(),
  /** Serveur MCP (HTTP, 127.0.0.1) pendant que l'interface est ouverte. */
  mcpServer: z.boolean().default(false),
  mcpPort: z.int().min(1_024).max(65_535).default(47_110),
});

export const ConfigSchema = PreferencesSchema.extend({
  connections: z.array(ConnectionSchema).default([]),
  profiles: z.array(ProfileSchema).default([]),
  /** Namespaces et workloads épinglés : `connexion/cluster/namespace[/workload]`. */
  favorites: z.array(z.string()).default([]),
  /** GitLab, Jira, Confluence, Rocket.Chat : URL et réglages (jetons : secrets.json). */
  services: ServicesSchema.default({}),
  /** Notifications GitLab / Jira / Confluence renvoyées vers Rocket.Chat. */
  notifications: NotificationsSchema.default(() => NotificationsSchema.parse({})),
  /** Mémoire de contexte IA (LangGraph, sans base vectorielle), exposée par le MCP. */
  contexte: ContextSchema.default(() => ContextSchema.parse({})),
  /** Filtres de logs enregistrés (nommés), réutilisables dans tous les onglets. */
  filterPresets: z.array(FilterPresetSchema).default([]),
  /** Accès web & contrôle de navigateur (outils MCP web_*), désactivé par défaut. */
  web: WebSchema.default(() => WebSchema.parse({})),
  /** Bases de données consultables (lecture seule) par les agents via les outils MCP bdd_*. */
  databases: z.array(DatabaseSchema).default([]),
});

export type Theme = z.infer<typeof ThemeSchema>;
export type TlsOptions = z.infer<typeof TlsSchema>;
export type Connection = z.infer<typeof ConnectionSchema>;
export type RancherConnection = z.infer<typeof RancherConnectionSchema>;
export type KafkaConnection = z.infer<typeof KafkaConnectionSchema>;
export type S3Connection = z.infer<typeof S3ConnectionSchema>;
export type ConnectionType = Connection['type'];
export type ServiceId = z.infer<typeof ServiceIdSchema>;
export type ServiceConfig = z.infer<typeof ServiceSchema>;
export type ServicesConfig = z.infer<typeof ServicesSchema>;
export type NotificationSource = z.infer<typeof NotificationSourceSchema>;
export type NotificationsConfig = z.infer<typeof NotificationsSchema>;
export type ContextSource = z.infer<typeof ContextSourceSchema>;
export type ContextConfig = z.infer<typeof ContextSchema>;
export type Profile = z.infer<typeof ProfileSchema>;
export type StoredFilterRule = z.infer<typeof StoredFilterRuleSchema>;
export type FilterPreset = z.infer<typeof FilterPresetSchema>;
export type Preferences = z.infer<typeof PreferencesSchema>;
export type WebConfig = z.infer<typeof WebSchema>;
export type DatabaseEngine = z.infer<typeof DatabaseEngineSchema>;
export type DatabaseConnection = z.infer<typeof DatabaseSchema>;
export type AppConfig = z.infer<typeof ConfigSchema>;
