import { age, size } from './format';

describe('tailles et âges', () => {
  it('taille', () => {
    expect(size(undefined)).toBe('');
    expect(size(512)).toBe('512 o');
    expect(size(1536)).toBe('1,5 Ko');
    expect(size(25 * 1024 * 1024)).toBe('25 Mo');
  });

  it('âge', () => {
    const now = Date.parse('2026-10-22T12:00:00Z');
    expect(age(undefined, now)).toBe('');
    expect(age('2026-10-22T11:55:00Z', now)).toBe('5 min');
    expect(age('2026-10-22T09:00:00Z', now)).toBe('3 h');
    expect(age('2026-10-19T12:00:00Z', now)).toBe('3 j');
  });
});
