import { wrapText, wrappedCell, wrappedValue } from './json-cell';
import { LogBuffer } from './log-buffer';

const entry = (text: string) => {
  const buffer = new LogBuffer(10);
  buffer.push([{ text }]);
  return buffer.entries[0];
};

describe('colonnes JSON indentées', () => {
  it('champ objet ou chaîne contenant du JSON : indenté ; sinon rien', () => {
    const e = entry('{"msg":"ok","ctx":{"user":42,"roles":["a"]},"raw":"{\\"x\\":1}","n":3}');
    expect(wrappedValue(e, 'ctx', {})).toBe('{\n  "user": 42,\n  "roles": [\n    "a"\n  ]\n}');
    expect(wrappedValue(e, 'raw', {})).toBe('{\n  "x": 1\n}');
    expect(wrappedValue(e, 'n', {})).toBeUndefined();
    expect(wrappedValue(e, 'absent', {})).toBeUndefined();
  });

  it('colonne message : champ message, sinon toute la ligne JSON', () => {
    expect(wrappedValue(entry('{"msg":"texte"}'), '@message', {})).toBeUndefined();
    expect(wrappedValue(entry('{"id":7,"etat":"payé"}'), '@message', {})).toBe(
      '{\n  "id": 7,\n  "etat": "payé"\n}',
    );
    expect(wrappedValue(entry('pas du json'), '@message', {})).toBeUndefined();
  });

  it('très long : tronqué', () => {
    const big = JSON.stringify({ list: Array.from({ length: 500 }, (_, i) => i) });
    const lines = wrappedValue(entry(big), '@message', {})!.split('\n');
    expect(lines).toHaveLength(201);
    expect(lines.at(-1)).toMatch(/ligne\(s\) de plus/);
  });
});

describe('colonnes à la ligne (texte coupé à la largeur)', () => {
  it('coupe après un espace, sinon au milieu d’un mot trop long', () => {
    expect(wrapText('aaa bbb ccc ddd', 8)).toBe('aaa bbb \nccc ddd');
    expect(wrapText('abcdefghij', 4)).toBe('abcd\nefgh\nij');
    expect(wrapText('court', 20)).toBe('court');
    expect(wrapText('a\tb', 20)).toBe('a    b');
    expect(wrapText('ligne 1\r\nligne 2', 20)).toBe('ligne 1\nligne 2');
  });

  it('borné : la suite est renvoyée au détail', () => {
    const text = 'x'.repeat(50);
    expect(wrapText(text, 10, 3).split('\n')).toEqual(['x'.repeat(10), 'x'.repeat(10), 'x'.repeat(10), '… (suite : voir le détail)']);
    expect(wrapText('x'.repeat(30), 10, 3).split('\n')).toHaveLength(3); // tout tient : pas de mention
  });

  it('log non JSON : le message est coupé à la largeur de la colonne', () => {
    const e = entry('2026-11-20 10:00:00 ERROR Connexion refusée par le serveur de base de données principal');
    const cell = wrappedCell(e, '@message', {}, 30)!;
    expect(cell.split('\n').every((l) => l.length <= 30)).toBe(true);
    expect(cell.replace(/\n/g, '')).toBe(e.parsed.message);
    expect(cell.split('\n').length).toBeGreaterThan(1);
  });

  it('JSON : indenté puis coupé ; champ texte coupé ; champ absent : rien', () => {
    const e = entry(`{"msg":"ok","ctx":{"user":"${'u'.repeat(40)}"},"path":"/api/v1/commandes/12345/lignes"}`);
    const ctx = wrappedCell(e, 'ctx', {}, 20)!;
    expect(ctx.startsWith('{\n')).toBe(true);
    expect(ctx.split('\n').every((l) => l.length <= 20)).toBe(true);
    expect(wrappedCell(e, 'path', {}, 12)).toBe('/api/v1/comm\nandes/12345/\nlignes');
    expect(wrappedCell(e, 'absent', {}, 12)).toBeUndefined();
  });
});
