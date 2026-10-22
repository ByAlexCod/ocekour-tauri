import type { Connection, SourceErrorInfo, SourceNode, SourcePath } from '../../../shared/api';
import { type NodeState, nodeKey } from './source-tree.service';

export type RowKind = 'connection' | 'cluster' | 'favorite' | 'group' | SourceNode['kind'];

export interface Row {
  key: string;
  target: SourcePath;
  depth: number;
  kind: RowKind;
  label: string;
  /** Texte secondaire (favori : cluster, namespace). */
  detail?: string;
  /** Favori dont la connexion ou le cluster n'existe plus. */
  missing?: boolean;
  /** Ligne de connexion : son type (badge). */
  connectionType?: Connection['type'];
  expandable: boolean;
  expanded: boolean;
  loading: boolean;
  error?: SourceErrorInfo;
  node?: SourceNode;
  favorite?: boolean;
  /** Clé de favori (namespace ou workload), si la ligne peut être épinglée. */
  favoriteKey?: string;
}

export interface TreeInput {
  connections: Connection[];
  nodes: ReadonlyMap<string, NodeState>;
  isExpanded: (target: SourcePath) => boolean;
  /** Section « Favoris » : favori déplié ? */
  isFavoriteOpen?: (favorite: string) => boolean;
  favorites: ReadonlySet<string>;
  filter: string;
}

/**
 * Aplatit l'arborescence en lignes affichables. Avec un filtre, une ligne est gardée si
 * elle correspond ou si l'un de ses descendants déjà chargés correspond (ses ancêtres
 * sont alors dépliés d'office).
 */
function builder(input: TreeInput) {
  const filter = input.filter.trim().toLowerCase();
  const matches = (label: string) => !filter || label.toLowerCase().includes(filter);

  const walk = (
    row: Omit<Row, 'expanded' | 'loading' | 'error'>,
    childrenOf: () => Row[][],
    open = input.isExpanded(row.target),
  ) => {
    // L'état de chargement est celui du nœud visé, même affiché ailleurs (favoris).
    const state = input.nodes.get(nodeKey(row.target));
    const expandedByUser = row.expandable && open;
    const children = row.expandable && (expandedByUser || filter) ? childrenOf().flat() : [];
    if (filter && !matches(row.label) && !children.length) return [];
    const expanded = expandedByUser || (!!filter && children.length > 0);
    const self: Row = {
      ...row,
      expanded,
      loading: !!state?.loading,
      error: state?.error,
    };
    return [self, ...(expanded ? children : [])];
  };

  const sourceChildren = (parent: SourcePath, depth: number, workload?: string): Row[][] => {
    const state = input.nodes.get(nodeKey(parent));
    const [clusterId, namespace] = parent.path;
    const base = `${parent.connectionId}/${clusterId}`;
    const favoriteKeyOf = (node: SourceNode) =>
      node.kind === 'topic' || node.kind === 'bucket'
        ? `${parent.connectionId}/${node.id}`
        : node.kind === 'namespace'
          ? `${base}/${node.id}`
          : node.kind === 'pod' && node.workload
            ? `${base}/${namespace}/${node.workload}`
            : undefined;

    return (
      (state?.children ?? [])
        .filter((node) => !workload || node.workload === workload)
        .map((node) => ({ node, favoriteKey: favoriteKeyOf(node) }))
        .map((n) => ({ ...n, favorite: !!n.favoriteKey && input.favorites.has(n.favoriteKey) }))
        // Favoris en tête, ordre d'origine sinon (tri stable).
        .sort((a, b) => Number(b.favorite) - Number(a.favorite))
        .map(({ node, favoriteKey, favorite }) => {
          const target = { connectionId: parent.connectionId, path: [...parent.path, node.id] };
          return walk(
            {
              key: nodeKey(target),
              target,
              depth,
              kind: node.kind,
              label: node.id,
              expandable: node.expandable,
              node,
              favorite,
              favoriteKey,
            },
            () => sourceChildren(target, depth + 1),
          );
        })
    );
  };

  /**
   * Topics : au-delà de 20, regroupés par préfixe (`commandes.creees` → `commandes`) ;
   * un préfixe isolé reste à plat.
   */
  const rootChildren = (connection: Connection): Row[][] => {
    const root: SourcePath = { connectionId: connection.id, path: [] };
    const rows = sourceChildren(root, 1);
    const nodes = input.nodes.get(nodeKey(root))?.children ?? [];
    if (connection.type !== 'kafka' || nodes.length <= 20) return rows;
    const prefixOf = (id: string) => /^([^._-]+)[._-]/.exec(id)?.[1];
    const counts = new Map<string, number>();
    for (const n of nodes) {
      const p = prefixOf(n.id);
      if (p) counts.set(p, (counts.get(p) ?? 0) + 1);
    }
    // 1re passe : membres de chaque groupe ; 2e passe : lignes dans l'ordre d'origine.
    const members = new Map<string, Row[][]>();
    nodes.forEach((node, i) => {
      const prefix = prefixOf(node.id);
      if (prefix && (counts.get(prefix) ?? 0) >= 2) {
        members.set(prefix, [...(members.get(prefix) ?? []), rows[i]]);
      }
    });
    const result: Row[][] = [];
    const done = new Set<string>();
    nodes.forEach((node, i) => {
      const prefix = prefixOf(node.id);
      const group = prefix ? members.get(prefix) : undefined;
      if (!group) {
        result.push(rows[i]);
        return;
      }
      if (done.has(prefix!)) return;
      done.add(prefix!);
      const target = { connectionId: connection.id, path: [`#${prefix}`] };
      result.push(
        walk(
          {
            key: nodeKey(target),
            target,
            depth: 1,
            kind: 'group',
            label: prefix!,
            detail: `${group.length} topics`,
            expandable: true,
          },
          () => group.map((m) => m.map((r) => ({ ...r, depth: r.depth + 1 }))),
        ),
      );
    });
    return result;
  };

  return { sourceChildren, walk, rootChildren };
}

/** Arborescence complète : connexion › cluster › namespace › pod › conteneur. */
export function treeRows(input: TreeInput): Row[] {
  const { sourceChildren, walk, rootChildren } = builder(input);
  return input.connections.flatMap((connection) => {
    const target = { connectionId: connection.id, path: [] };
    return walk(
      {
        key: nodeKey(target),
        target,
        depth: 0,
        kind: 'connection',
        label: connection.name,
        connectionType: connection.type,
        expandable: true,
      },
      () =>
        connection.type !== 'rancher'
          ? rootChildren(connection)
          : connection.clusters.map((cluster) => {
              const clusterTarget = { connectionId: connection.id, path: [cluster.id] };
              return walk(
                {
                  key: nodeKey(clusterTarget),
                  target: clusterTarget,
                  depth: 1,
                  kind: 'cluster',
                  label: cluster.name,
                  expandable: true,
                },
                () => sourceChildren(clusterTarget, 2),
              );
            }),
    );
  });
}

/**
 * Section « Favoris » : chaque favori est une racine qui se déplie directement sur les
 * pods du namespace, ou sur les pods actuels du workload (leurs noms changent à chaque
 * déploiement, le workload reste).
 */
export function favoriteRows(input: TreeInput): Row[] {
  const { sourceChildren, walk } = builder(input);
  // Triés par nom affiché (workload ou namespace), pas par clé interne.
  const byLabel = (key: string) => key.split('/').slice(2).reverse().join('/');
  const sorted = [...input.favorites].sort((a, b) => byLabel(a).localeCompare(byLabel(b)));
  return sorted.flatMap((favorite): Row[] => {
    const [connectionId, clusterId, namespace, workload] = favorite.split('/');
    const connection = input.connections.find((c) => c.id === connectionId);
    // Topic ou bucket : `connexion/<nom>`, ouvert directement.
    if (connection && connection.type !== 'rancher') {
      const leaf = clusterId;
      const target: SourcePath = { connectionId, path: [leaf] };
      return [
        {
          key: `★${favorite}`,
          target,
          depth: 0,
          kind: connection.type === 'kafka' ? 'topic' : 'bucket',
          label: leaf,
          detail: connection.name,
          expandable: false,
          expanded: false,
          loading: false,
          favorite: true,
          favoriteKey: favorite,
        },
      ];
    }
    if (!connection || connection.type !== 'rancher') {
      return [
        {
          key: `★${favorite}`,
          target: { connectionId, path: [] },
          depth: 0,
          kind: 'favorite',
          label: favorite.split('/').slice(1).join('/'),
          missing: true,
          expandable: false,
          expanded: false,
          loading: false,
          favorite: true,
          favoriteKey: favorite,
        },
      ];
    }
    const cluster = connection.clusters.find((c) => c.id === clusterId);
    const target: SourcePath = { connectionId, path: [clusterId, namespace] };
    const row = {
      key: `★${favorite}`,
      target,
      depth: 0,
      kind: 'favorite' as const,
      label: workload ?? namespace,
      detail: workload
        ? `${cluster?.name ?? clusterId} · ${namespace}`
        : (cluster?.name ?? clusterId),
      missing: !cluster,
      expandable: !!cluster,
      favorite: true,
      favoriteKey: favorite,
    };
    return walk(
      row,
      () => sourceChildren(target, 1, workload),
      !!cluster && !!input.isFavoriteOpen?.(favorite),
    );
  });
}
