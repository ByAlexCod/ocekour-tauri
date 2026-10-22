import { exportContent, exportFileName, exportLine } from './export';
import { LogBuffer } from './log-buffer';

describe('export', () => {
  const buffer = new LogBuffer(10);
  buffer.push([
    { ts: '2026-09-24T10:00:00.000Z', text: '{"level":"info","msg":"ok"}' },
    { ts: '2026-09-24T10:00:01.000Z', text: 'texte simple' },
    {
      ts: '2026-09-24T10:00:02.000Z',
      text: '{"id":7}',
      meta: { partition: 1, offset: '42', key: 'k', headers: { h: 'v' }, format: 'json' },
    },
  ]);
  buffer.notice('Reconnexion');
  const [json, plain, kafka] = buffer.entries;

  it('NDJSON : JSON d’origine, texte enveloppé, message Kafka complet', () => {
    expect(exportLine(json, 'ndjson')).toBe('{"level":"info","msg":"ok"}');
    expect(JSON.parse(exportLine(plain, 'ndjson'))).toEqual({
      time: '2026-09-24T10:00:01.000Z',
      message: 'texte simple',
    });
    expect(JSON.parse(exportLine(kafka, 'ndjson'))).toEqual({
      timestamp: '2026-09-24T10:00:02.000Z',
      partition: 1,
      offset: '42',
      key: 'k',
      headers: { h: 'v' },
      value: { id: 7 },
    });
  });

  it('texte brut, sans les événements d’Ocekour', () => {
    expect(exportContent(buffer.entries, 'text')).toBe(
      '{"level":"info","msg":"ok"}\ntexte simple\n2026-09-24T10:00:02.000Z p1@42 k {"id":7}\n',
    );
  });

  it('assemblage : la source préfixe le texte et figure dans le NDJSON', () => {
    const b = new LogBuffer(10);
    const src = { id: 'm1', label: 'api', color: '#0ea5e9' };
    b.pushTagged({ ts: '2026-09-24T10:00:00.000Z', text: 'bonjour' }, {}, src);
    const [entry] = b.entries;
    expect(exportLine(entry, 'text')).toBe('[api] bonjour');
    expect(JSON.parse(exportLine(entry, 'ndjson'))).toEqual({
      source: 'api',
      time: '2026-09-24T10:00:00.000Z',
      log: 'bonjour',
    });
  });

  it('nom de fichier valide', () => {
    expect(exportFileName('api/app: 1', new Date(2026, 8, 24, 9, 5))).toBe(
      'api_app_ 1-20260924-0905',
    );
  });
});
