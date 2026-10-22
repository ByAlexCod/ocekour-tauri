import type { SourceNode, SourcePath } from '../../../shared/api';

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

export type StreamTarget = ContainerTarget | TopicTarget;

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
  }
}

/** Libellé court d'une cible de flux (badge de source, onglet). */
export function streamLabel(t: StreamTarget): string {
  return t.kind === 'container' ? t.container : t.topic;
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
  }
}

/** Clé des profils de config.yaml : `cluster/namespace/workload` ou `kafka/<connexion>/<topic>`. */
export function profileKey(t: StreamTarget): string {
  return t.kind === 'container'
    ? `${t.clusterName}/${t.namespace}/${t.workload}`
    : `kafka/${t.connectionName}/${t.topic}`;
}

/** Chemin côté main (`streams.open`). */
export function streamPath(t: StreamTarget): SourcePath {
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
