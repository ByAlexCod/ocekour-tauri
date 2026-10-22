import { buildMatcher } from './filter';
import { LogBuffer } from './log-buffer';
import { LogRows } from './log-rows';

const line = (level: string, msg: string) => ({ text: JSON.stringify({ level, msg }) });

describe('LogBuffer', () => {
  it('retire les plus anciennes en tête et tient les comptes par niveau', () => {
    const buffer = new LogBuffer(3);
    const added = buffer.push([line('info', 'a'), line('error', 'b'), line('info', 'c')]);
    expect(added.map((e) => e.seq)).toEqual([0, 1, 2]);
    expect(buffer.levelCounts.info).toBe(2);
    buffer.push([line('warn', 'd'), line('warn', 'e')]);
    expect(buffer.length).toBe(3);
    expect(buffer.firstSeq).toBe(2);
    expect(buffer.dropped).toBe(2);
    expect(buffer.levelCounts).toMatchObject({ info: 1, error: 0, warn: 2 });
    expect(buffer.entries.map((e) => e.parsed.message)).toEqual(['c', 'd', 'e']);
  });

  it('ne ré-analyse que si les champs clés changent', () => {
    const buffer = new LogBuffer(10);
    buffer.push([{ text: JSON.stringify({ sev: 'error', msg: 'x' }) }]);
    expect(buffer.levelCounts.unknown).toBe(1);
    expect(buffer.setFields({})).toBe(false);
    expect(buffer.setFields({ level: 'sev' })).toBe(true);
    expect(buffer.levelCounts).toMatchObject({ unknown: 0, error: 1 });
    expect(buffer.setFields({ level: 'sev' })).toBe(false);
  });

  it('ré-ordonne par horodatage et renumérote (assemblage : source ajoutée)', () => {
    const buffer = new LogBuffer(10);
    const src = { id: 'm', label: 'm', color: '#000' };
    // Une source déjà présente, puis une source ajoutée dont l'historique est antérieur.
    buffer.pushTagged({ text: 'récent', ts: '2026-01-01T00:00:03Z' }, {}, src);
    buffer.pushTagged({ text: 'ancien-1', ts: '2026-01-01T00:00:01Z' }, {}, src);
    buffer.pushTagged({ text: 'ancien-2', ts: '2026-01-01T00:00:02Z' }, {}, src);
    buffer.reorderByTime();
    expect(buffer.entries.map((e) => e.text)).toEqual(['ancien-1', 'ancien-2', 'récent']);
    expect(buffer.entries.map((e) => e.seq)).toEqual([0, 1, 2]);
  });

  it('compacte sans perdre de lignes au-delà de milliers de retraits', () => {
    const buffer = new LogBuffer(1_000);
    for (let i = 0; i < 20; i++) {
      buffer.push(Array.from({ length: 1_000 }, (_, j) => line('info', `${i}-${j}`)));
    }
    expect(buffer.length).toBe(1_000);
    expect(buffer.at(0)?.seq).toBe(19_000);
    expect(buffer.entries[999].parsed.message).toBe('19-999');
    expect(buffer.levelCounts.info).toBe(1_000);
  });
});

describe('LogRows', () => {
  it('filtre de façon incrémentale et suit les retraits du tampon', () => {
    const buffer = new LogBuffer(3);
    const rows = new LogRows(20);
    rows.reset(buffer, { levels: new Set(['error']) });
    rows.append(buffer.push([line('info', 'a'), line('error', 'b')]));
    expect(rows.length).toBe(1);
    rows.append(buffer.push([line('error', 'c'), line('error', 'd'), line('info', 'e')]));
    rows.trimBefore(buffer.firstSeq);
    expect(rows.slice().map((e) => e.parsed.message)).toEqual(['c', 'd']);
    expect(rows.indexOfSeq(3)).toBe(1);
    expect(rows.indexOfSeq(1)).toBe(-1);
    expect(rows.total).toBe(40);
  });

  it('compte les correspondances en mode « surligner »', () => {
    const buffer = new LogBuffer(3);
    const rows = new LogRows(20);
    const highlight = buildMatcher({ text: 'paiement', caseSensitive: false, regex: false });
    rows.reset(buffer, { highlight });
    rows.append(buffer.push([line('info', 'Paiement ok'), line('info', 'autre')]));
    expect(rows.matchCount).toBe(1);
    rows.append(buffer.push([line('info', 'x'), line('info', 'y')]));
    rows.trimBefore(buffer.firstSeq);
    expect(rows.matchCount).toBe(0);
    expect(rows.find(0, 1, (e) => e.parsed.message === 'y')).toBe(2);
  });

  it('hauteurs variables : positions cumulées, y compris après retrait en tête', () => {
    const buffer = new LogBuffer(10_000);
    const rows = new LogRows(20);
    rows.setHeights((e) => (e.parsed.message.startsWith('grand') ? 50 : 20), buffer);
    rows.append(buffer.push([line('info', 'a'), line('info', 'grand'), line('info', 'b')]));
    expect(rows.total).toBe(90);
    expect(rows.offsetOf(2)).toBe(70);
    expect(rows.heightOf(1)).toBe(50);
    expect(rows.indexAt(69)).toBe(1);
    expect(rows.indexAt(70)).toBe(2);
    rows.trimBefore(1);
    expect(rows.total).toBe(70);
    expect(rows.offsetOf(1)).toBe(50);
    expect(rows.indexAt(10)).toBe(0);
  });
});
