import { wrappedValue } from './json-cell';
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
