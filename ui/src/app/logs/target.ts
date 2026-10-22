import { type FileEncoding, LOCAL_CONNECTION, type SourceNode, type SourcePath, type StreamOptions } from '../../../shared/api';

// Ce qu'un onglet affiche. Les conteneurs et les topics sont des flux (liste de lignes) ;
// les buckets s'ouvrent dans l'explorateur.

export interface ContainerTarget {
  kind: 'container';
  connectionId: string;
  clusterId: string;
  namespace: string;
  pod: string;
  container: string;
  node: SourceNode;
  /** Nom lisible du cluster et workload du pod : clé des profils (`cluster/namespace/workload`). */
  clusterName: string;
  workload: string;
}

export interface TopicTarget {
  kind: 'topic';
  connectionId: string;
  connectionName: string;
  topic: string;
}

export interface BucketTarget {
  kind: 'bucket';
  connectionId: string;
  connectionName: string;
  bucket: string;
}

/** Fichier de logs : du poste (`connectionId` absent) ou exposé par une sonde. */
export interface FileTarget {
  kind: 'file';
  /** Absent : fichier du poste ; sinon connexion de type sonde. */
  connectionId?: string;
  connectionName?: string;
  /** Poste : [fichier ou motif, fichier du motif ?] ; sonde : [source, fichier du motif ?]. */
  path: string[];
  /** Nom affiché (fichier, motif, ou source de la sonde). */
  name: string;
  /** Motif (ou source de type motif) suivi sans fichier précis : le plus récent. */
  pattern?: boolean;
  /** Encodage imposé (fichier du poste). */
  encoding?: FileEncoding;
}

export type StreamTarget = ContainerTarget | TopicTarget | FileTarget;

/** Un membre d'un assemblage : une cible de flux, avec son étiquette et sa couleur. */
export interface AssemblyMember {
  /** Id stable dans l'assemblage (couleur, filtre, étiquette). */
  id: string;
  target: StreamTarget;
  label: string;
  color: string;
}

/** Onglet d'assemblage : plusieurs flux fusionnés dans une vue triée dans le temps. */
export interface AssemblyTarget {
  kind: 'assembly';
  /** Id stable de l'assemblage (clé d'onglet, lien de partage). */
  id: string;
  name?: string;
  members: AssemblyMember[];
}

export type TabTarget = StreamTarget | BucketTarget | AssemblyTarget;

/** Au plus, sources d'un assemblage (bornage réseau/CPU). */
export const MAX_ASSEMBLY_MEMBERS = 10;

/** Palette de couleurs des membres (teintes distinctes, lisibles en clair et sombre). */
export const ASSEMBLY_COLORS = [
  '#0ea5e9',
  '#f59e0b',
  '#10b981',
  '#ef4444',
  '#8b5cf6',
  '#ec4899',
  '#14b8a6',
  '#f97316',
  '#6366f1',
  '#84cc16',
];

/** Identifiant unique : un même conteneur, topic ou bucket n'est ouvert qu'une fois. */
export function targetKey(t: TabTarget): string {
  switch (t.kind) {
    case 'container':
      return [t.connectionId, t.clusterId, t.namespace, t.pod, t.container].join('/');
    case 'topic':
      return `${t.connectionId}/${t.topic}`;
    case 'bucket':
      return `${t.connectionId}/${t.bucket}`;
    case 'assembly':
      return `assembly/${t.id}`;
    case 'file':
      return `file/${t.connectionId ?? LOCAL_CONNECTION}/${t.path.join('/')}`;
  }
}

/** Connexion d'une cible (`local` pour un fichier du poste). */
export function connectionIdOf(t: StreamTarget | BucketTarget): string {
  return t.kind === 'file' ? (t.connectionId ?? LOCAL_CONNECTION) : t.connectionId;
}

/** Dernier segment d'un chemin (Windows ou Unix). */
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** Libellé court d'une cible de flux (badge de source, onglet). */
export function streamLabel(t: StreamTarget): string {
  return t.kind === 'container' ? t.container : t.kind === 'topic' ? t.topic : fileTitle(t);
}

/** Titre d'un onglet de fichier : le fichier choisi dans un motif, sinon le nom. */
function fileTitle(t: FileTarget): string {
  return t.path.length > 1 ? t.path[t.path.length - 1] : t.name;
}

/** Titre court (onglet) et complément (pod, connexion). */
export function targetTitle(t: TabTarget): { title: string; subtitle: string } {
  switch (t.kind) {
    case 'container':
      return { title: t.container, subtitle: t.pod };
    case 'topic':
      return { title: t.topic, subtitle: t.connectionName };
    case 'bucket':
      return { title: t.bucket, subtitle: t.connectionName };
    case 'assembly':
      return {
        title: t.name || `Assemblage (${t.members.length})`,
        subtitle: `${t.members.length} source${t.members.length > 1 ? 's' : ''}`,
      };
    case 'file':
      return { title: fileTitle(t), subtitle: t.connectionName ?? 'fichier local' };
  }
}

/** Chemin complet, pour les infobulles et l'en-tête de la vue. */
export function targetPath(t: TabTarget): string[] {
  switch (t.kind) {
    case 'container':
      return [t.clusterName, t.namespace, t.pod, t.container];
    case 'topic':
      return [t.connectionName, t.topic];
    case 'bucket':
      return [t.connectionName, t.bucket];
    case 'assembly':
      return [t.name || `Assemblage (${t.members.length})`];
    case 'file':
      return t.connectionId ? [t.connectionName ?? t.connectionId, ...t.path] : t.path;
  }
}

/** Nom de fichier sans date ni numéro de rotation (profil commun à tous les jours). */
export function fileProfileName(name: string): string {
  return name
    .replace(/\.gz$/, '')
    .replace(/[-_.]\*/g, '')
    .replace(/\*/g, '')
    .replace(/[-_.]?\d{4}-\d\d-\d\d|[-_.]?\d{8}|\.\d+$|-\d+(\.)/g, '$1');
}

/** Clé des profils de config.yaml : `cluster/namespace/workload` ou `kafka/<connexion>/<topic>`. */
export function profileKey(t: StreamTarget): string {
  switch (t.kind) {
    case 'container':
      return `${t.clusterName}/${t.namespace}/${t.workload}`;
    case 'topic':
      return `kafka/${t.connectionName}/${t.topic}`;
    case 'file':
      // Mêmes clés que le cœur (outils MCP).
      return t.connectionId ? `sonde/${t.connectionName}/${t.path[0]}` : `fichier/${fileProfileName(baseName(t.path[t.path.length - 1]))}`;
  }
}

/** Options d'ouverture d'un flux : l'encodage choisi pour un fichier en plus. */
export function streamOptions(t: StreamTarget, options: StreamOptions): StreamOptions {
  return t.kind === 'file' && t.encoding ? { ...options, encoding: t.encoding } : options;
}

/** Chemin côté main (`streams.open`). */
export function streamPath(t: StreamTarget): SourcePath {
  if (t.kind === 'file') return { connectionId: t.connectionId ?? LOCAL_CONNECTION, path: t.path };
  return t.kind === 'container'
    ? {
        connectionId: t.connectionId,
        // Le workload sert à proposer les pods actuels si ce pod n'existe plus.
        path: [t.clusterId, t.namespace, t.pod, t.container, ...(t.workload ? [t.workload] : [])],
      }
    : { connectionId: t.connectionId, path: [t.topic] };
}

/** Anciennes sessions sauvegardées (avant les types d'onglets) : ce sont des conteneurs. */
export function migrateTarget(t: TabTarget | Omit<ContainerTarget, 'kind'>): TabTarget {
  return 'kind' in t ? t : { ...t, kind: 'container' };
}
