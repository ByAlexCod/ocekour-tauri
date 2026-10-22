import {
  expandNested,
  getFieldDeep,
  normalizeLevel,
  oneLine,
  parseLine,
  stripAnsi,
  toDate,
} from './parser';

const K8S_TS = '2026-09-24T08:00:00.5Z';

describe('parseLine', () => {
  it('Logback JSON : heure, niveau, message', () => {
    const p = parseLine(
      JSON.stringify({
        '@timestamp': '2026-09-24T10:00:00.123+02:00',
        level: 'WARN',
        logger_name: 'fr.acme.X',
        message: 'Stock bas',
        stack_trace: 'a\nb',
      }),
      K8S_TS,
    );
    expect(p.level).toBe('warn');
    expect(p.message).toBe('Stock bas');
    expect(p.time?.toISOString()).toBe('2026-09-24T08:00:00.123Z');
    expect(p.json?.['logger_name']).toBe('fr.acme.X');
  });

  it('pino : niveau numérique et epoch en ms', () => {
    const p = parseLine('{"level":50,"time":1790245591136,"msg":"Échec"}');
    expect(p).toMatchObject({ level: 'error', message: 'Échec' });
    expect(p.time?.getTime()).toBe(1790245591136);
  });

  it('Serilog compact (@t, @l, @mt) et champs imbriqués (log.level)', () => {
    expect(parseLine('{"@t":"2026-09-24T10:00:00Z","@l":"Error","@mt":"Boom"}')).toMatchObject({
      level: 'error',
      message: 'Boom',
    });
    expect(parseLine('{"log":{"level":"debug"},"message":"x"}').level).toBe('debug');
  });

  it('champs imposés par le profil', () => {
    const p = parseLine('{"sev":"E","txt":"custom","at":"2026-01-01T00:00:00Z"}', undefined, {
      level: 'sev',
      message: 'txt',
      time: 'at',
    });
    expect(p).toMatchObject({ level: 'error', message: 'custom' });
    expect(p.time?.toISOString()).toBe('2026-01-01T00:00:00.000Z');
  });

  it('JSON sans message : la ligne entière, et horodatage Kubernetes en repli', () => {
    const p = parseLine('{"a":1}', K8S_TS);
    expect(p.message).toBe('{"a":1}');
    expect(p.time?.toISOString()).toBe('2026-09-24T08:00:00.500Z');
    expect(p.level).toBe('unknown');
  });

  it('texte : niveau par mot-clé ou logfmt, codes ANSI retirés', () => {
    expect(parseLine('2026-09-24 10:00:00 ERROR [main] Boom').level).toBe('error');
    expect(parseLine('time=… level=warning msg="disk"').level).toBe('warn');
    expect(parseLine('\x1b[32mINFO\x1b[0m démarré')).toMatchObject({
      level: 'info',
      message: 'INFO démarré',
    });
    expect(parseLine('[2026] "GET /api HTTP/1.1" 200').level).toBe('unknown');
  });

  it('JSON invalide ou tableau → texte', () => {
    expect(parseLine('{"a":').json).toBeUndefined();
    expect(parseLine('[1,2]').json).toBeUndefined();
  });
});

describe('normalizeLevel', () => {
  it.each([
    ['WARNING', 'warn'],
    ['W', 'warn'],
    ['Information', 'info'],
    ['CRITICAL', 'fatal'],
    [30, 'info'],
    [60, 'fatal'],
    [3, 'error'],
    [7, 'debug'],
    ['40', 'warn'],
    ['banane', 'unknown'],
    [undefined, 'unknown'],
  ])('%s → %s', (input, expected) => expect(normalizeLevel(input)).toBe(expected));
});

describe('toDate', () => {
  it('Logback, epoch s/ms/ns', () => {
    expect(toDate('2026-09-24 10:00:00,123')?.getMilliseconds()).toBe(123);
    expect(toDate(1790245591)?.getTime()).toBe(1790245591000);
    expect(toDate(1790245591136)?.getTime()).toBe(1790245591136);
    expect(toDate('1790245591136000000')?.getTime()).toBe(1790245591136);
    expect(toDate('pas une date')).toBeUndefined();
  });
});

describe('utilitaires', () => {
  it('oneLine rend les retours à la ligne visibles', () => {
    expect(oneLine('a\nb\tc')).toBe('a ⏎ b c');
  });

  it('stripAnsi', () => {
    expect(stripAnsi('\x1b[1;31mrouge\x1b[0m')).toBe('rouge');
  });

  it('expandNested désérialise le JSON contenu dans des chaînes', () => {
    const input = { payload: '{"client":{"id":1},"lignes":"[1,2]"}', texte: '{pas du json' };
    expect(expandNested(input)).toEqual({
      payload: { client: { id: 1 }, lignes: [1, 2] },
      texte: '{pas du json',
    });
    expect(input.payload).toBe('{"client":{"id":1},"lignes":"[1,2]"}');
  });
});

describe('getFieldDeep', () => {
  it('traverse les chaînes JSON', () => {
    const obj = { payload: '{"client":{"id":7}}', 'a.b': 1, c: { d: 2 } };
    expect(getFieldDeep(obj, 'payload.client.id')).toBe(7);
    expect(getFieldDeep(obj, 'a.b')).toBe(1);
    expect(getFieldDeep(obj, 'c.d')).toBe(2);
    expect(getFieldDeep(obj, 'payload.absent')).toBeUndefined();
  });
});
