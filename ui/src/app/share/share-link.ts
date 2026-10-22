import * as z from 'zod/mini';
import type { Connection } from '../../../shared/api';
import type { TabState } from '../logs/tab-state';
import type { TabTarget } from '../logs/target';

// Lien de partage `ocekour://open?d=<base64url JSON>` : de quoi rouvrir la même vue chez
// un collègue (source, cible, options, filtres). JAMAIS de secret ni d'id local : la
// connexion est retrouvée par son URL (Rancher, S3, sonde) ou ses brokers (Kafka). Un
// fichier du poste n'est pas partageable (chemin propre à la machine).
// `zod/mini` : quelques Ko dans l'interface, au lieu de ~450 Ko pour zod complet.

const PREFIX = 'ocekour://open?d=';

const Options = z.object({
  follow: z.boolean(),
  tailLines: z.optional(z.int().check(z.minimum(0))),
  sinceSeconds: z.optional(z.int().check(z.minimum(1))),
  previous: z.optional(z.boolean()),
});

const Filters = z.object({
  text: z.string().check(z.maxLength(2_000)),
  caseSensitive: z.boolean(),
  regex: z.boolean(),
  mode: z.enum(['filter', 'highlight']),
  levels: z.optional(
    z.array(z.enum(['trace', 'debug', 'info', 'warn', 'error', 'fatal', 'unknown'])),
  ),
});

const Name = z.string().check(z.minLength(1), z.maxLength(512));

export const ShareLinkSchema = z.discriminatedUnion('t', [
  z.object({
    v: z.literal(1),
    t: z.literal('rancher'),
    url: z.url(),
    clusterId: Name,
    clusterName: Name,
    namespace: Name,
    pod: Name,
    container: Name,
    workload: z.string().check(z.maxLength(512)),
    options: Options,
    filters: z.optional(Filters),
  }),
  z.object({
    v: z.literal(1),
    t: z.literal('kafka'),
    brokers: z.array(Name).check(z.minLength(1), z.maxLength(50)),
    topic: Name,
    options: Options,
    filters: z.optional(Filters),
  }),
  z.object({
    v: z.literal(1),
    t: z.literal('sonde'),
    url: z.url(),
    source: Name,
    file: z.optional(Name),
    options: Options,
    filters: z.optional(Filters),
  }),
  z.object({
    v: z.literal(1),
    t: z.literal('s3'),
    url: z.url(),
    bucket: Name,
    prefix: z.string().check(z.maxLength(1024)),
  }),
]);

export type ShareLink = z.infer<typeof ShareLinkSchema>;

function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function fromBase64Url(data: string): string {
  const binary = atob(data.replace(/-/g, '+').replace(/_/g, '/'));
  return new TextDecoder().decode(Uint8Array.from(binary, (c) => c.charCodeAt(0)));
}

export function encodeLink(link: ShareLink): string {
  return PREFIX + toBase64Url(JSON.stringify(link));
}

/** Le texte contient-il un lien Ocekour ? (collage) */
export function findLink(text: string): string | undefined {
  return text.match(/ocekour:\/\/open\?d=[A-Za-z0-9_-]+/)?.[0];
}

/** Lien collé → contenu validé ; lève une erreur lisible sinon. */
export function decodeLink(text: string): ShareLink {
  const link = findLink(text.trim());
  if (!link) throw new Error("Ce texte n'est pas un lien Ocekour (ocekour://open?d=…).");
  let json: unknown;
  try {
    json = JSON.parse(fromBase64Url(link.slice(PREFIX.length)));
  } catch {
    throw new Error('Lien Ocekour illisible (copié en entier ?).');
  }
  if ((json as { v?: unknown })?.v !== 1) {
    throw new Error('Lien créé par une autre version d’Ocekour : mettez l’application à jour.');
  }
  const parsed = ShareLinkSchema.safeParse(json);
  if (!parsed.success) throw new Error('Lien Ocekour invalide (contenu incomplet).');
  return parsed.data;
}

const normalizeUrl = (url: string) => url.trim().replace(/\/+$/, '').toLowerCase();

/** Lien d'un onglet (état courant : options, filtres, dossier). */
export function linkFor(state: TabState, connection: Connection): ShareLink {
  const { target } = state;
  const filters = state.search && {
    text: state.search.text,
    caseSensitive: state.search.caseSensitive,
    regex: state.search.regex,
    mode: state.searchMode ?? 'filter',
    levels: state.levels,
  };
  const options = state.options ?? { follow: true };
  if (target.kind === 'container' && connection.type === 'rancher') {
    return {
      v: 1,
      t: 'rancher',
      url: connection.url,
      clusterId: target.clusterId,
      clusterName: target.clusterName,
      namespace: target.namespace,
      pod: target.pod,
      container: target.container,
      workload: target.workload,
      options,
      filters,
    };
  }
  if (target.kind === 'file' && connection.type === 'sonde') {
    return {
      v: 1,
      t: 'sonde',
      url: connection.url,
      source: target.path[0],
      ...(target.path[1] ? { file: target.path[1] } : {}),
      options,
      filters,
    };
  }
  if (target.kind === 'topic' && connection.type === 'kafka') {
    return { v: 1, t: 'kafka', brokers: connection.brokers, topic: target.topic, options, filters };
  }
  if (target.kind === 'bucket' && connection.type === 's3') {
    return {
      v: 1,
      t: 's3',
      url: connection.url,
      bucket: target.bucket,
      prefix: state.prefix ?? '',
    };
  }
  throw new Error('Onglet et connexion incohérents.');
}

/** Connexion locale qui correspond au lien (même URL, ou un broker en commun). */
export function findConnection(link: ShareLink, connections: Connection[]): Connection | undefined {
  if (link.t === 'kafka') {
    const brokers = new Set(link.brokers.map((b) => b.trim().toLowerCase()));
    return connections.find(
      (c) => c.type === 'kafka' && c.brokers.some((b) => brokers.has(b.trim().toLowerCase())),
    );
  }
  const url = normalizeUrl(link.url);
  return connections.find((c) => c.type === link.t && normalizeUrl(c.url) === url);
}

/** État d'onglet à ouvrir pour ce lien, avec la connexion locale trouvée. */
export function stateFor(link: ShareLink, connection: Connection): TabState {
  const filters = 'filters' in link ? link.filters : undefined;
  const search = filters && {
    text: filters.text,
    caseSensitive: filters.caseSensitive,
    regex: filters.regex,
  };
  let target: TabTarget;
  switch (link.t) {
    case 'rancher':
      target = {
        kind: 'container',
        connectionId: connection.id,
        clusterId: link.clusterId,
        clusterName: link.clusterName,
        namespace: link.namespace,
        pod: link.pod,
        container: link.container,
        workload: link.workload,
        node: { id: link.container, kind: 'container', expandable: false },
      };
      break;
    case 'sonde':
      target = {
        kind: 'file',
        connectionId: connection.id,
        connectionName: connection.name,
        path: link.file ? [link.source, link.file] : [link.source],
        name: link.source,
      };
      break;
    case 'kafka':
      target = {
        kind: 'topic',
        connectionId: connection.id,
        connectionName: connection.name,
        topic: link.topic,
      };
      break;
    case 's3':
      return {
        target: {
          kind: 'bucket',
          connectionId: connection.id,
          connectionName: connection.name,
          bucket: link.bucket,
        },
        prefix: link.prefix,
      };
  }
  return {
    target,
    options: link.options,
    search,
    searchMode: filters?.mode,
    levels: filters?.levels,
  };
}
