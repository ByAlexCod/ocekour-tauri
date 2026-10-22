import { jsonMatches } from './json-node';

describe('recherche dans le détail', () => {
  it('cherche dans les clés et les valeurs, en profondeur', () => {
    const value = { user: { id: 42, name: 'Alice' }, tags: ['prod'] };
    expect(jsonMatches(null, value, '')).toBe(true);
    expect(jsonMatches(null, value, 'alice')).toBe(true);
    expect(jsonMatches(null, value, 'prod')).toBe(true);
    expect(jsonMatches(null, value, 'id')).toBe(true);
    expect(jsonMatches(null, value, 'bob')).toBe(false);
    expect(jsonMatches('clé', null, 'cl')).toBe(true);
  });
});
