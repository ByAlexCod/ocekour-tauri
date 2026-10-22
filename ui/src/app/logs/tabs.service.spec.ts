import type { SourceNode } from '../../../shared/api';
import type { LogSession } from './log-session';
import type { ContainerTarget } from './target';
import { TabsService } from './tabs.service';

const node: SourceNode = { id: 'app', kind: 'container', expandable: false };
const target = (pod: string, container = 'app'): ContainerTarget => ({
  kind: 'container',
  connectionId: 'r1',
  clusterId: 'c1',
  clusterName: 'prod',
  namespace: 'ns',
  workload: 'api',
  pod,
  container,
  node,
});

describe('TabsService', () => {
  let opened: string[];
  let closed: string[];

  beforeEach(() => {
    opened = [];
    closed = [];
    localStorage.clear();
    let n = 0;
    window.ocekour = {
      streams: {
        open: async (t: { path: string[] }) => {
          opened.push(t.path[2]);
          return `s${n++}`;
        },
        close: async (id: string) => {
          closed.push(id);
        },
        onEvent: () => () => {},
      },
    } as unknown as Window['ocekour'];
  });

  const labels = (tabs: TabsService) => tabs.tabs().map((t) => (t.target as ContainerTarget).pod);

  it('clic : remplace l’onglet courant ; Ctrl+clic : nouvel onglet ; déjà ouvert : activé', () => {
    const tabs = new TabsService();
    tabs.open(target('p1'));
    tabs.open(target('p2'));
    expect(labels(tabs)).toEqual(['p2']);
    tabs.open(target('p3'), true);
    expect(labels(tabs)).toEqual(['p2', 'p3']);
    tabs.open(target('p2'));
    expect((tabs.active()?.target as ContainerTarget).pod).toBe('p2');
    expect(labels(tabs)).toEqual(['p2', 'p3']);
  });

  it('fermer, parcourir, déplacer', () => {
    const tabs = new TabsService();
    for (const pod of ['a', 'b', 'c']) tabs.open(target(pod), true);
    expect((tabs.active()?.target as ContainerTarget).pod).toBe('c');
    tabs.cycle(1);
    expect((tabs.active()?.target as ContainerTarget).pod).toBe('a');
    tabs.move(0, 2);
    expect(labels(tabs)).toEqual(['b', 'c', 'a']);
    tabs.close();
    expect(labels(tabs)).toEqual(['b', 'c']);
    expect((tabs.active()?.target as ContainerTarget).pod).toBe('c');
  });

  it('remplacement de pod : même position, options conservées', () => {
    const tabs = new TabsService();
    tabs.open(target('a'), true);
    tabs.open(target('b'), true);
    const first = tabs.tabs()[0] as LogSession;
    first.options.set({ follow: false, tailLines: 100 });
    tabs.replaceTarget(first, target('a2'));
    expect(labels(tabs)).toEqual(['a2', 'b']);
    expect((tabs.tabs()[0] as LogSession).options()).toEqual({ follow: false, tailLines: 100 });
  });
});

describe('TabsService — onglets explorateur', () => {
  it('un bucket s’ouvre dans un onglet explorateur, sans flux', () => {
    window.ocekour = {
      streams: { open: async () => 's', close: async () => {}, onEvent: () => () => {} },
    } as unknown as Window['ocekour'];
    const tabs = new TabsService();
    tabs.open({ kind: 'bucket', connectionId: 's1', connectionName: 'S3', bucket: 'logs' });
    expect(tabs.active()?.kind).toBe('explorer');
    expect(tabs.tabs()).toHaveLength(1);
  });
});
