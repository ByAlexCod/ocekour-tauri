import { buildMatcher, segments } from './filter';

describe('buildMatcher', () => {
  it('texte insensible à la casse par défaut, caractères spéciaux littéraux', () => {
    const m = buildMatcher({ text: 'a.b', caseSensitive: false, regex: false })!;
    expect(m.test('xx A.B yy')).toBe(true);
    expect(m.test('axb')).toBe(false);
  });

  it('sensible à la casse', () => {
    const m = buildMatcher({ text: 'Error', caseSensitive: true, regex: false })!;
    expect(m.test('error')).toBe(false);
    expect(m.test('Error')).toBe(true);
  });

  it('expression régulière, plages multiples, appels répétés stables', () => {
    const m = buildMatcher({ text: 'commande \\d+', caseSensitive: false, regex: true })!;
    const text = 'Commande 12 et commande 345';
    expect(m.ranges(text)).toEqual([
      [0, 11],
      [15, 27],
    ]);
    expect(m.test(text)).toBe(true);
    expect(m.test(text)).toBe(true);
  });

  it('regex invalide → erreur lisible ; recherche vide → aucun matcher', () => {
    expect(() => buildMatcher({ text: '(', caseSensitive: false, regex: true })).toThrow(
      /Expression régulière invalide/,
    );
    expect(buildMatcher({ text: '', caseSensitive: false, regex: false })).toBeUndefined();
  });

  it('regex correspondant à la chaîne vide : pas de boucle infinie', () => {
    const m = buildMatcher({ text: 'x*', caseSensitive: false, regex: true })!;
    expect(m.ranges('abxxc')).toEqual([[2, 4]]);
  });
});

describe('segments', () => {
  it('découpe en morceaux surlignés', () => {
    const m = buildMatcher({ text: 'ab', caseSensitive: false, regex: false });
    expect(segments('xAbyab', m)).toEqual([
      { text: 'x', match: false },
      { text: 'Ab', match: true },
      { text: 'y', match: false },
      { text: 'ab', match: true },
    ]);
    expect(segments('rien', undefined)).toEqual([{ text: 'rien', match: false }]);
  });
});
