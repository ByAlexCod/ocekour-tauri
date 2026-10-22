// Contrat IPC entre le process Electron (main/preload) et le front Angular.
// Import uniquement en `import type` : ce fichier ne produit aucun code.

import type {
  AppConfig,
  ContextConfig,
  ContextSource,
  NotificationSource,
  NotificationsConfig,
  DatabaseEngine,
  Preferences,
  ServiceId,
  StoredFilterRule,
  TlsOptions,
} from './config';

export type {
  AppConfig,
  Connection,
  ConnectionType,
  KafkaConnection,
  Preferences,
  Profile,
  RancherConnection,
  S3Connection,
  ServiceConfig,
  ServiceId,
  NotificationSource,
  NotificationsConfig,
  ContextSource,
  ContextConfig,
  FilterPreset,
  StoredFilterRule,
  DatabaseEngine,
  DatabaseConnection,
  Theme,
  TlsOptions,
} from './config';

/** Base de données configurée, telle qu'exposée à l'interface (sans mot de passe). */
export interface DatabaseInfo {
  id: string;
  engine: DatabaseEngine;
  name: string;
  host: string;
  port: number;
  database?: string;
  user: string;
  ssl: boolean;
  hasSecret: boolean;
}

/** Brouillon de la fenêtre « Base de données » (connexion + mot de passe facultatif). */
export interface DatabaseDraft {
  id?: string;
  engine: DatabaseEngine;
  name?: string;
  host: string;
  port?: number;
  database?: string;
  user: string;
  ssl?: boolean;
  oracleSid?: boolean;
  tls?: TlsOptions;
  password?: string;
}

export interface DatabaseTestResult {
  ok: boolean;
  /** Résumé (version du serveur) si la connexion a réussi. */
  summary?: string;
  error?: string;
}

/** Objet partiel en profondeur (patch de configuration). */
export type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

/** Modification d'un profil de workload depuis l'interface (« en un clic »). */
export type ProfileEdit =
  | { op: 'toggleExpand'; path: string }
  | { op: 'toggleColumn'; path: string }
  | { op: 'toggleWrap'; path: string }
  | { op: 'setField'; role: 'time' | 'level' | 'message'; path: string | null }
  | { op: 'setParseNested'; value: boolean };

/** Erreur de config.yaml, positionnée si possible (lignes et colonnes à partir de 1). */
export interface ConfigError {
  message: string;
  line?: number;
  col?: number;
}

export interface ConfigState {
  /** Dernière configuration valide (celle qui est appliquée). */
  config: AppConfig;
  /** Erreurs du fichier sur disque ; vide s'il est valide. */
  errors: ConfigError[];
  path: string;
}

export type SaveResult = { ok: true; state: ConfigState } | { ok: false; errors: ConfigError[] };

interface DraftBase {
  id?: string;
  name: string;
  /** Token, mot de passe ou clé secrète ; absent = garder celui enregistré pour `id`. */
  secret?: string;
  tls: TlsOptions;
}

export interface RancherDraft extends DraftBase {
  type: 'rancher';
  url: string;
  clusters: { id: string; name: string }[];
}

export interface KafkaDraft extends DraftBase {
  type: 'kafka';
  brokers: string[];
  security: {
    protocol: 'SASL_PLAINTEXT' | 'SASL_SSL' | 'PLAINTEXT';
    mechanism: 'SCRAM-SHA-512' | 'SCRAM-SHA-256' | 'PLAIN';
    username?: string;
  };
  registry?: { url: string; tls: TlsOptions };
}

export interface S3Draft extends DraftBase {
  type: 's3';
  url: string;
  region?: string;
  pathStyle: boolean;
  accessKeyId: string;
  buckets: string[];
}

/** Connexion en cours de saisie dans la fenêtre de connexion. */
export type ConnectionDraft = RancherDraft | KafkaDraft | S3Draft;

export type SourceErrorCode =
  'invalid' | 'unauthorized' | 'forbidden' | 'tls' | 'network' | 'timeout' | 'not-rancher' | 'http' | 'not-found';

export interface SourceErrorInfo {
  code: SourceErrorCode;
  message: string;
}

/** Résultat d'un appel réseau : l'erreur est typée pour que l'UI propose la bonne action. */
export type Result<T> = { ok: true; value: T } | { ok: false; error: SourceErrorInfo };

export interface RancherCluster {
  id: string;
  name: string;
  state: string;
}

export interface CaFileInfo {
  path: string;
  subject: string;
  validTo: string;
  expired: boolean;
  /** Nombre de certificats dans le fichier (un bundle PEM peut en contenir plusieurs). */
  count: number;
}

export type NodeKind = 'namespace' | 'pod' | 'container' | 'topic' | 'bucket';

/** Élément de l'arborescence d'une source (sous le niveau cluster). */
export interface SourceNode {
  /** Segment de chemin (nom du namespace, du pod ou du conteneur). */
  id: string;
  kind: NodeKind;
  /** `false` pour un conteneur : c'est lui qu'on ouvre. */
  expandable: boolean;
  /** Santé : ok (vert), warn (orange), error (rouge), off (gris : terminé). */
  health?: 'ok' | 'warn' | 'error' | 'off';
  /** État lisible : Running, CrashLoopBackOff, Terminating… */
  state?: string;
  /** Pod : workload propriétaire (Deployment, StatefulSet…). */
  workload?: string;
  restarts?: number;
  /** Date de création (ISO), pour l'âge. */
  createdAt?: string;
  /** Conteneur d'initialisation. */
  init?: boolean;
  /** Les logs du conteneur précédent existent (redémarrage). */
  hasPrevious?: boolean;
  /** Topic Kafka : nombre de partitions. */
  partitions?: number;
}

/** Chemin d'un nœud : connexion, puis cluster › namespace › pod › conteneur. */
export interface SourcePath {
  connectionId: string;
  path: string[];
}

/** Métadonnées d'un message Kafka. */
export interface KafkaMeta {
  partition: number;
  /** Offset (chaîne : les offsets Kafka sont des entiers 64 bits). */
  offset: string;
  key?: string;
  headers?: Record<string, string>;
  /** Format détecté de la valeur. */
  format: 'avro' | 'json' | 'text' | 'hex' | 'null';
  /** Problème de décodage (registre injoignable, schéma inconnu…). */
  warning?: string;
}

/**
 * Une ligne brute : ligne de log (`ts` = horodatage Kubernetes) ou message Kafka
 * (`ts` = horodatage du message, `text` = valeur décodée, `meta` = partition, offset, clé…).
 */
export interface LogLine {
  ts?: string;
  text: string;
  meta?: KafkaMeta;
}

export interface StreamOptions {
  /** Suivi en temps réel ; sinon historique seul. */
  follow: boolean;
  /** N dernières lignes ; absent = tout. */
  tailLines?: number;
  /** Depuis N secondes. */
  sinceSeconds?: number;
  /** Logs de l'instance précédente du conteneur (après un redémarrage). */
  previous?: boolean;
}

export type StreamState = 'connecting' | 'live' | 'reconnecting' | 'ended' | 'error';

export type StreamEvent =
  | { type: 'lines'; lines: LogLine[] }
  | {
      type: 'status';
      state: StreamState;
      message?: string;
      error?: SourceErrorInfo;
      /** Pod supprimé : pods actuels du même workload. */
      replacements?: string[];
    }
  /** Événement à insérer dans le flux (redémarrage, reconnexion…). */
  | { type: 'notice'; message: string };

/** Bucket S3 visé par l'explorateur. */
export interface S3Ref {
  connectionId: string;
  bucket: string;
}

/** Entrée d'un dossier S3 (délimiteur `/`). `key` = clé complète (dossier : se termine par `/`). */
export interface S3Entry {
  key: string;
  name: string;
  kind: 'folder' | 'file';
  size?: number;
  /** ISO 8601. */
  modified?: string;
}

export interface S3Page {
  entries: S3Entry[];
  /** Jeton pour la page suivante (pagination S3). */
  next?: string;
}

/** Aperçu d'un fichier : texte (début si gros), image, ou rien d'affichable. */
export interface S3Preview {
  kind: 'text' | 'image' | 'binary';
  /** Texte décodé (UTF-8), éventuellement décompressé (.gz). */
  text?: string;
  /** Image : `data:` URL. */
  dataUrl?: string;
  size: number;
  contentType?: string;
  /** Seul le début du fichier est affiché (limite d'aperçu). */
  truncated: boolean;
  gunzipped?: boolean;
}

/** Progression d'une opération longue (téléchargement, envoi, suppression, renommage). */
export interface S3Progress {
  opId: string;
  /** Objets traités / total (total inconnu pendant l'inventaire). */
  done: number;
  total?: number;
  bytes?: number;
  current?: string;
  finished?: boolean;
  cancelled?: boolean;
  error?: SourceErrorInfo;
}

/** Clé d'activation (sans la clé elle-même ni la configuration Matomo). */
export interface LicenceInfo {
  id: string;
  label: string;
  issuedAt: string;
  mode: 'none' | 'optional' | 'required';
}

/**
 * État de l'activation : `missing` (écran d'activation), `checking` (suivi obligatoire :
 * Matomo contacté), `active`, `blocked` (suivi obligatoire, Matomo injoignable).
 */
export interface LicenceState {
  status: 'missing' | 'checking' | 'active' | 'blocked';
  info?: LicenceInfo;
  message?: string;
  /** Build de développement (accepte les clés de dev). */
  development?: boolean;
  /** Réponse à l'activation : préconfiguration de la clé ajoutée à config.yaml. */
  preset?: { connections: string[]; profiles: number; skipped: string[]; services?: string[] };
}

/** Compte d'un service (GitLab, Jira…) tel que vu par l'écran Comptes : jamais le jeton. */
export interface ServiceAccount {
  id: ServiceId;
  label: string;
  /** Absent : service non configuré. */
  url?: string;
  tls: TlsOptions;
  enabled: boolean;
  write: boolean;
  hasToken: boolean;
  /** Le jeton est un couple identifiant + jeton (Rocket.Chat). */
  needsUserId: boolean;
}

/** Résultat du test d'un jeton. */
export interface ServiceTestResult {
  ok: boolean;
  /** « Alex Martin (@amartin) ». */
  identity?: string;
  /** Avertissement non bloquant (portée insuffisante, expiration…). */
  warning?: string;
  error?: string;
}

/** Une source à télécharger (flux simple, ou un membre d'un assemblage). */
export interface LogDownloadSource {
  connectionId: string;
  /** Rancher : [cluster, namespace, pod, conteneur, workload?] ; Kafka : [topic]. */
  path: string[];
  /** Assemblage : libellé de la source (préfixe dans l'export). */
  label?: string;
  /** Inclure aussi les logs de l'instance précédente du conteneur (avant redémarrage). */
  includePrevious?: boolean;
}

/** Téléchargement de l'historique complet (depuis la création) d'un flux ou d'un assemblage. */
export interface LogDownloadRequest {
  /** Nom de fichier proposé (sans extension ni date). */
  defaultName: string;
  /** Appliquer les filtres courants (recherche + niveaux + sources masquées) ; sinon tout. */
  filtered: boolean;
  search?: { text: string; regex: boolean; caseSensitive: boolean };
  /** Niveaux gardés (vide/absent = tous). */
  levels?: string[];
  sources: LogDownloadSource[];
}

export interface LogDownloadResult {
  file: string;
  /** Lignes écrites. */
  lines: number;
  /** Historique tronqué (limite atteinte). */
  truncated: boolean;
}

/** Écriture faite par un agent via le MCP (journal local). */
export interface AgentAction {
  id: string;
  /** ISO 8601. */
  date: string;
  /** Client MCP déclaré (`claude-code 2.1.0`), si connu. */
  client?: string;
  service: ServiceId;
  tool: string;
  /** Élément visé, lisible (« groupe/projet !12 », « PAY-123 »). */
  target: string;
  /** Lien web vers le résultat (commentaire, ticket, page…). */
  url?: string;
  params: Record<string, unknown>;
  status: 'ok' | 'error';
  error?: string;
}

export interface AgentActionQuery {
  offset?: number;
  limit?: number;
  service?: ServiceId;
  status?: 'ok' | 'error';
  text?: string;
}

/** État du scrutateur de notifications. */
export interface NotifierStatus {
  enabled: boolean;
  running: boolean;
  intervalSeconds: number;
  target: string;
  /** Sources activées ET munies d'un jeton. */
  actives: NotificationSource[];
  /** Dernier passage (ISO 8601). */
  lastRun?: string;
  /** Nouveautés envoyées au dernier passage. */
  lastCount?: number;
  lastError?: string;
}

/** État du moteur de contexte IA. */
export interface ContextStatus {
  enabled: boolean;
  running: boolean;
  intervalSeconds: number;
  /** Sources activées ET joignables (URL + jeton). */
  sources: ContextSource[];
  /** Entités connues dans la mémoire. */
  entities: number;
  /** LLM de curation retenu (« qwen · qwen2.5 »), ou absent (stockage brut). */
  llm?: string;
  lastRun?: string;
  lastCount?: number;
  lastError?: string;
}

export type AgentTool = 'qwen' | 'vibe';

/** État de l'intégration MCP chez un agent local (Qwen Code, Mistral Vibe). */
export interface AgentState {
  tool: AgentTool;
  /** L'outil est présent sur le poste (dossier de config). */
  present: boolean;
  /** Ocekour est déjà enregistré comme serveur MCP. */
  configured: boolean;
  /** Chemin du fichier de configuration. */
  file: string;
}

/** Serveur MCP intégré. */
export interface McpStatus {
  enabled: boolean;
  running: boolean;
  url: string;
  token: string;
  /** Chemin de l'exe, pour le mode sans interface (`--mcp`, `--mcp-http`). */
  executable: string;
  error?: string;
}

export interface OcekourApi {
  config: {
    get(): Promise<ConfigState>;
    setPreferences(patch: Partial<Preferences>): Promise<ConfigState>;
    /** Modifie les réglages de notifications (fusion partielle). */
    setNotifications(patch: DeepPartial<NotificationsConfig>): Promise<ConfigState>;
    /** Modifie les réglages du contexte IA (fusion partielle). */
    setContexte(patch: DeepPartial<ContextConfig>): Promise<ConfigState>;
    readText(): Promise<string>;
    validateText(text: string): Promise<ConfigError[]>;
    saveText(text: string): Promise<SaveResult>;
    /** Épingle ou désépingle un namespace / workload (`connexion/cluster/namespace[/workload]`). */
    toggleFavorite(key: string): Promise<ConfigState>;
    /** Modifie (ou crée en tête de liste) le profil dont `match` est exactement celui donné. */
    editProfile(match: string, edit: ProfileEdit): Promise<ConfigState>;
    /** Enregistre (ou remplace) un filtre nommé, réutilisable dans tous les onglets. */
    saveFilterPreset(name: string, rules: StoredFilterRule[]): Promise<ConfigState>;
    /** Supprime le filtre enregistré de ce nom. */
    deleteFilterPreset(name: string): Promise<ConfigState>;
    /** Renomme un filtre enregistré (refusé si le nouveau nom est déjà pris). */
    renameFilterPreset(name: string, to: string): Promise<ConfigState>;
    /** Appelé à chaque rechargement (écriture par l'app ou modification externe). */
    onChanged(listener: (state: ConfigState) => void): () => void;
  };
  connections: {
    /** Vérifie la connexion ; renvoie un résumé lisible (« Connecté en tant que … »). */
    test(draft: ConnectionDraft): Promise<Result<{ summary: string }>>;
    listClusters(draft: RancherDraft): Promise<Result<RancherCluster[]>>;
    /** Crée ou met à jour ; le secret est chiffré à part. Renvoie l'id. */
    save(draft: ConnectionDraft): Promise<string>;
    delete(id: string): Promise<void>;
    hasSecret(id: string): Promise<boolean>;
    /** Boîte de dialogue « Ouvrir » pour un .cer/.crt/.pem ; `null` si annulé. */
    pickCaFile(): Promise<CaFileInfo | null>;
    readCaFile(path: string): Promise<CaFileInfo>;
    /** Boîte de dialogue « Ouvrir » pour un kubeconfig Rancher ; `null` si annulé. */
    importKubeconfig(): Promise<RancherDraft | null>;
  };
  databases: {
    /** Liste les bases configurées (sans mot de passe). */
    list(): Promise<DatabaseInfo[]>;
    /** Teste la connexion (mot de passe du brouillon, sinon celui enregistré). */
    test(draft: DatabaseDraft): Promise<DatabaseTestResult>;
    /** Crée ou met à jour ; le mot de passe est chiffré à part. Renvoie l'id. */
    save(draft: DatabaseDraft): Promise<string>;
    delete(id: string): Promise<void>;
  };
  sources: {
    /** Enfants d'un nœud ; `path` commence au cluster (`[clusterId]`). */
    browse(target: SourcePath): Promise<Result<SourceNode[]>>;
  };
  streams: {
    /** Ouvre un flux sur un conteneur (`path` = cluster, namespace, pod, conteneur). */
    open(target: SourcePath, options: StreamOptions): Promise<string>;
    close(streamId: string): Promise<void>;
    onEvent(listener: (streamId: string, event: StreamEvent) => void): () => void;
  };
  s3: {
    /** Contenu d'un dossier ; `search` filtre par début de nom (préfixe S3). */
    list(
      ref: S3Ref,
      prefix: string,
      options?: { search?: string; continuation?: string },
    ): Promise<Result<S3Page>>;
    preview(ref: S3Ref, key: string): Promise<Result<S3Preview>>;
    /** Nombre d'objets sous un dossier (confirmation avant suppression / renommage). */
    count(ref: S3Ref, prefix: string): Promise<Result<number>>;
    /**
     * Opérations longues : renvoient un `opId` (progression via `onProgress`), ou `null`
     * si l'utilisateur annule la boîte de dialogue.
     */
    download(ref: S3Ref, entries: S3Entry[]): Promise<string | null>;
    /**
     * Envoi dans `prefix` : chemins locaux (glisser-déposer), ou boîte « Ouvrir » pour
     * choisir des fichiers (`'files'`) ou un dossier (`'folder'`).
     */
    upload(
      ref: S3Ref,
      prefix: string,
      source: string[] | 'files' | 'folder',
    ): Promise<string | null>;
    rename(ref: S3Ref, entry: S3Entry, newName: string): Promise<string>;
    delete(ref: S3Ref, entries: S3Entry[]): Promise<string>;
    cancel(opId: string): Promise<void>;
    onProgress(listener: (progress: S3Progress) => void): () => void;
    /** Chemin local d'un fichier glissé depuis l'Explorateur Windows. */
    pathForFile(file: File): string;
  };
  /** Copie du texte dans le presse-papiers du système. */
  copyText(text: string): Promise<void>;
  licence: {
    state(): Promise<LicenceState>;
    /** Vérifie et enregistre la clé collée ; erreur lisible si elle est refusée. */
    activate(key: string): Promise<Result<LicenceState>>;
    retry(): Promise<LicenceState>;
    onChanged(listener: (state: LicenceState) => void): () => void;
  };
  analytics: {
    /** Écran affiché : `path` sans donnée identifiante (`logs/kafka`, `parametres`…). */
    page(title: string, path: string): Promise<void>;
    /** Action anonyme : catégorie, action, nom (jamais de nom de serveur, topic, bucket…). */
    event(event: {
      category: string;
      action: string;
      name?: string;
      value?: number;
    }): Promise<void>;
    /** Statistiques envoyées (clé avec suivi) ; la destination n'est jamais exposée. */
    status(): Promise<{ enabled: boolean }>;
  };
  services: {
    list(): Promise<ServiceAccount[]>;
    /** Enregistre (ou retire avec `url` vide) l'URL et le TLS d'un service. */
    save(id: ServiceId, settings: { url: string; tls: TlsOptions }): Promise<ServiceAccount[]>;
    setFlags(
      id: ServiceId,
      flags: { enabled?: boolean; write?: boolean },
    ): Promise<ServiceAccount[]>;
    /** Enregistre le jeton (chiffré) puis le teste. */
    setToken(id: ServiceId, token: string, userId?: string): Promise<ServiceTestResult>;
    test(id: ServiceId): Promise<ServiceTestResult>;
    forgetToken(id: ServiceId): Promise<ServiceAccount[]>;
    /** Ouvre la page de création de jeton du serveur dans le navigateur. */
    openTokenPage(id: ServiceId, fallback?: boolean): Promise<void>;
  };
  actions: {
    list(query: AgentActionQuery): Promise<{ entries: AgentAction[]; total: number }>;
    /** Ouvre un lien http(s) dans le navigateur (lien d'une action). */
    openUrl(url: string): Promise<void>;
    onAdded(listener: (action: AgentAction) => void): () => void;
  };
  notifications: {
    status(): Promise<NotifierStatus>;
    /** Envoie un message d'essai à la cible configurée. */
    test(): Promise<{ ok: boolean; error?: string }>;
    onChanged(listener: (status: NotifierStatus) => void): () => void;
  };
  contexte: {
    status(): Promise<ContextStatus>;
    /** LLM de curation détecté pour le mode courant (« qwen · modèle »), ou `null`. */
    detectLlm(): Promise<string | null>;
    /** État de l'intégration MCP chez Qwen Code et Mistral Vibe. */
    agents(): Promise<AgentState[]>;
    /** Ajoute ou retire Ocekour du serveur MCP d'un agent (écrit dans sa config). */
    configureAgent(tool: AgentTool, action: 'add' | 'remove'): Promise<AgentState[]>;
    onChanged(listener: (status: ContextStatus) => void): () => void;
  };
  mcp: {
    /** Serveur MCP HTTP : état, adresse et jeton (pour configurer un client). */
    status(): Promise<McpStatus>;
    /** Nouveau jeton (l'ancien ne fonctionne plus). */
    regenerateToken(): Promise<McpStatus>;
  };
  /** Écrit une erreur de l'interface dans le journal (`logs\main.log`). */
  logError(message: string): Promise<void>;
  /** Texte du presse-papiers (bouton « Ouvrir un lien »). */
  readClipboard(): Promise<string>;
  export: {
    /** Boîte « Enregistrer sous » (NDJSON ou texte) ; `null` si annulé. */
    pick(defaultName: string): Promise<{ token: string; format: 'ndjson' | 'text' } | null>;
    /** Écrit dans le fichier choisi par `pick` (et seulement celui-là). */
    write(token: string, content: string): Promise<void>;
    /**
     * Télécharge l'historique complet (depuis la création) d'un flux ou d'un assemblage :
     * boîte « Enregistrer sous », récupération côté main, écriture. `null` si annulé.
     */
    downloadLogs(request: LogDownloadRequest): Promise<LogDownloadResult | null>;
  };
  /** Ouvre l'Explorateur sur config.yaml. */
  showConfigFile(): Promise<void>;
  /** Ouvre l'Explorateur sur le journal (`logs\main.log`). */
  showLogFile(): Promise<void>;
}
