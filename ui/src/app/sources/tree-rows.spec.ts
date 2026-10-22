import type { Connection, SourceNode, SourcePath } from '../../../shared/api';
import type { NodeState } from './source-tree.service';
import { favoriteRows, treeRows } from './tree-rows';

const connection: Connection = {
  id: 'r1',
  type: 'rancher',
  name: 'Rancher',
  url: 'https://rancher',
  tls: { insecure: false },
  clusters: [{ id: 'c1', name: 'prod' }],
};

const ns = (id: string): SourceNode => ({ id, kind: 'namespace', expandable: true });
const pod = (id: string, workload: string): SourceNode => ({
  id,
  kind: 'pod',
  expandable: true,
  workload,
});

const nodes = new Map<string, NodeState>([
  ['r1/c1', { loading: false, children: [ns('alpha'), ns('beta'), ns('gamma')] }],
  ['r1/c1/beta', { loading: false, children: [pod('api-1', 'api'), pod('front-1', 'front')] }],
]);

const rows = (options: { expanded?: string[]; favorites?: string[]; filter?: string }) =>
  treeRows({
    connections: [connection],
    nodes,
    isExpanded: (t: SourcePath) =>
      t.path.length
        ? (options.expanded ?? []).includes([t.connectionId, ...t.path].join('/'))
        : true,
    favorites: new Set(options.favorites ?? []),
    filter: options.filter ?? '',
  }).map((r) => `${'  '.repeat(r.depth)}${r.favorite ? '★' : ''}${r.label}`);

describe('treeRows', () => {
  it('affiche seulement les nœuds dépliés', () => {
    expect(rows({ expanded: ['r1/c1'] })).toEqual([
      'Rancher',
      '  prod',
      '    alpha',
      '    beta',
      '    gamma',
    ]);
  });

  it('met les favoris (namespace, workload) en tête', () => {
    expect(
      rows({ expanded: ['r1/c1', 'r1/c1/beta'], favorites: ['r1/c1/gamma', 'r1/c1/beta/front'] }),
    ).toEqual([
      'Rancher',
      '  prod',
      '    ★gamma',
      '    alpha',
      '    beta',
      '      ★front-1',
      '      api-1',
    ]);
  });

  it('filtre en gardant les ancêtres, même repliés', () => {
    expect(rows({ filter: 'FRONT' })).toEqual(['Rancher', '  prod', '    beta', '      front-1']);
  });
});

describe('favoriteRows', () => {
  const favRows = (open: string[], favorites: string[], filter = '') =>
    favoriteRows({
      connections: [connection],
      nodes,
      isExpanded: () => false,
      isFavoriteOpen: (f) => open.includes(f),
      favorites: new Set(favorites),
      filter,
    }).map(
      (r) =>
        `${'  '.repeat(r.depth)}${r.label}${r.detail ? ` (${r.detail})` : ''}${r.missing ? ' ✗' : ''}`,
    );

  it('racines toujours visibles, même si l’arbre est replié', () => {
    expect(favRows([], ['r1/c1/beta/front', 'r1/c1/beta', 'r1/c1/alpha'])).toEqual([
      'alpha (prod)',
      'beta (prod)',
      'front (prod · beta)',
    ]);
  });

  it('un workload favori se déplie sur ses seuls pods', () => {
    expect(favRows(['r1/c1/beta/front'], ['r1/c1/beta/front'])).toEqual([
      'front (prod · beta)',
      '  front-1',
    ]);
    expect(favRows(['r1/c1/beta'], ['r1/c1/beta'])).toEqual([
      'beta (prod)',
      '  api-1',
      '  front-1',
    ]);
  });

  it('favori dont le cluster a disparu : visible et marqué', () => {
    expect(favRows([], ['r1/c9/ns'])).toEqual(['ns (c9) ✗']);
  });
});

describe('treeRows — Kafka et S3', () => {
  const kafka: Connection = {
    id: 'k1',
    type: 'kafka',
    name: 'Kafka',
    brokers: ['k:9092'],
    security: { protocol: 'SASL_PLAINTEXT', mechanism: 'SCRAM-SHA-512' },
    tls: { insecure: false },
  };
  const topic = (id: string): SourceNode => ({ id, kind: 'topic', expandable: false });
  const topics = [
    ...Array.from({ length: 12 }, (_, i) => topic(`commandes.evt${i}`)),
    ...Array.from({ length: 9 }, (_, i) => topic(`stock-${i}`)),
    topic('seul.topic'),
  ];
  const kafkaRows = (expanded: string[], favorites: string[] = []) =>
    treeRows({
      connections: [kafka],
      nodes: new Map([['k1', { loading: false, children: topics }]]),
      isExpanded: (t) =>
        t.path.length ? expanded.includes([t.connectionId, ...t.path].join('/')) : true,
      favorites: new Set(favorites),
      filter: '',
    });

  it('au-delà de 20 topics, regroupement par préfixe (préfixe isolé à plat)', () => {
    const rows = kafkaRows([]);
    expect(rows.map((r) => `${r.kind}:${r.label}`)).toEqual([
      'connection:Kafka',
      'group:commandes',
      'group:stock',
      'topic:seul.topic',
    ]);
    expect(rows[0].connectionType).toBe('kafka');
  });

  it('un groupe déplié montre ses topics', () => {
    const rows = kafkaRows(['k1/#stock']);
    expect(rows.filter((r) => r.kind === 'topic').map((r) => r.label)).toContain('stock-3');
    expect(rows.find((r) => r.label === 'stock-3')?.depth).toBe(2);
  });

  it('un topic favori apparaît dans les favoris, ouvert directement', () => {
    const favs = favoriteRows({
      connections: [kafka],
      nodes: new Map(),
      isExpanded: () => false,
      favorites: new Set(['k1/commandes.evt3']),
      filter: '',
    });
    expect(favs).toMatchObject([
      { kind: 'topic', label: 'commandes.evt3', detail: 'Kafka', expandable: false },
    ]);
  });
});
