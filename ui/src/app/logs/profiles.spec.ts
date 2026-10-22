import type { Profile } from '../../../shared/api';
import { globToRegExp, resolveProfile } from './profiles';

const profile = (match: string, extra: Partial<Profile> = {}): Profile => ({
  match,
  fields: {},
  columns: [],
  expand: [],
  wrap: [],
  ...extra,
});

describe('profils', () => {
  it('jokers', () => {
    expect(globToRegExp('prod/*/api-*').test('prod/app-commandes/api-commandes')).toBe(true);
    expect(globToRegExp('prod/*').test('prod/app/api')).toBe(false);
    expect(globToRegExp('prod/**').test('prod/app/api')).toBe(true);
    expect(globToRegExp('recette.v2/*/*').test('recetteXv2/a/b')).toBe(false);
  });

  it('fusionne les profils correspondants, le premier prime', () => {
    const resolved = resolveProfile(
      [
        profile('prod/app-commandes/api-commandes', {
          expand: ['stack_trace'],
          fields: { message: 'msg' },
          wrap: ['@message'],
        }),
        profile('prod/*/*', {
          columns: ['traceId'],
          parseNested: true,
          expand: ['stack_trace', 'sql'],
          wrap: ['@message', 'context'],
        }),
        profile('recette/*/*', { columns: ['ignoré'] }),
        profile('*', { fields: { message: 'autre', level: 'sev' }, parseNested: false }),
      ],
      'prod/app-commandes/api-commandes',
    );
    expect(resolved).toEqual({
      key: 'prod/app-commandes/api-commandes',
      matches: ['prod/app-commandes/api-commandes', 'prod/*/*', '*'],
      fields: { message: 'msg', level: 'sev', time: undefined },
      columns: ['traceId'],
      expand: ['stack_trace', 'sql'],
      wrap: ['@message', 'context'],
      parseNested: true,
    });
  });

  it('aucun profil : valeurs par défaut', () => {
    expect(resolveProfile([], 'a/b/c')).toMatchObject({ matches: [], parseNested: false });
  });
});
