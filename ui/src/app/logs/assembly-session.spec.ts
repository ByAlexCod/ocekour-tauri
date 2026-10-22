import { AssemblySession, type StreamBinder } from './assembly-session';
import type { AssemblyTarget, StreamTarget } from './target';

const container: StreamTarget = {
  kind: 'container',
  connectionId: 'c1',
  clusterId: 'k',
  namespace: 'ns',
  pod: 'p',
  container: 'app',
  node: { id: 'app', kind: 'container', expandable: false },
  clusterName: 'K',
  workload: 'app',
};
const topic: StreamTarget = {
  kind: 'topic',
  connectionId: 'k1',
  connectionName: 'Kafka',
  topic: 'orders',
};

const target: AssemblyTarget = {
  kind: 'assembly',
  id: 'a1',
  members: [
    { id: 'm1', target: container, label: 'app', color: '#0ea5e9' },
    { id: 'm2', target: topic, label: 'orders', color: '#f59e0b' },
  ],
};

const line = (level: string, msg: string, ts: string) => ({
  text: JSON.stringify({ level, msg }),
  ts,
});

function makeSession() {
  const bound = new Map<string, AssemblySession>();
  let n = 0;
  const api = {
    open: async () => `s${++n}`,
    close: async () => undefined,
    onEvent: () => () => undefined,
  } as unknown as StreamBinder['api'];
  const binder: StreamBinder = {
    api,
    bind: (id, s) => bound.set(id, s),
    unbind: (id) => bound.delete(id),
  };
  const session = new AssemblySession(target, { follow: true }, 100, binder, () => ({}));
  session.reorderMs.set(0); // insertion immédiate (ordre d'arrivée), pas de minuterie
  return { session, bound };
}

describe('AssemblySession', () => {
  it('étiquette chaque ligne de sa source et les fusionne dans un tampon', async () => {
    const { session } = makeSession();
    await session.start();
    const [m1, m2] = session.members();
    session.handle({ type: 'lines', lines: [line('info', 'a', '2026-01-01T00:00:01Z')] }, m1.streamId);
    session.handle({ type: 'lines', lines: [line('error', 'b', '2026-01-01T00:00:02Z')] }, m2.streamId);
    const rows = session.rows.toArray();
    expect(rows.length).toBe(2);
    expect(rows.map((e) => e.source?.label)).toEqual(['app', 'orders']);
    expect(rows[0].source?.color).toBe('#0ea5e9');
  });

  it('masque une source via le filtre sans la retirer', async () => {
    const { session } = makeSession();
    await session.start();
    const [m1, m2] = session.members();
    session.handle({ type: 'lines', lines: [line('info', 'a', '2026-01-01T00:00:01Z')] }, m1.streamId);
    session.handle({ type: 'lines', lines: [line('info', 'b', '2026-01-01T00:00:02Z')] }, m2.streamId);
    session.toggleSource('m1');
    const rows = session.rows.toArray();
    expect(rows.length).toBe(1);
    expect(rows[0].source?.label).toBe('orders');
  });

  it('retire une source : flux délié et lignes masquées', async () => {
    const { session, bound } = makeSession();
    await session.start();
    const [m1] = session.members();
    session.handle({ type: 'lines', lines: [line('info', 'a', '2026-01-01T00:00:01Z')] }, m1.streamId);
    expect(bound.size).toBe(2);
    session.removeMember('m1');
    expect(session.members().length).toBe(1);
    expect(bound.size).toBe(1); // le flux de m1 est délié
    expect(session.rows.toArray().length).toBe(0); // sa ligne est masquée
  });
});
