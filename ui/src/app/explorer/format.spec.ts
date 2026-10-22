import type { S3Entry } from '../../../shared/api';
import { breadcrumbs, formatSize, previewLanguage, prettyText, sortEntries } from './format';

const file = (name: string, size: number, modified: string): S3Entry => ({
  key: name,
  name,
  kind: 'file',
  size,
  modified,
});

describe('format explorateur', () => {
  it('trie les dossiers d’abord, puis par nom naturel, taille ou date', () => {
    const entries: S3Entry[] = [
      file('f10.log', 5, '2026-01-02'),
      { key: 'z/', name: 'z', kind: 'folder' },
      file('f2.log', 50, '2026-01-01'),
      { key: 'a/', name: 'a', kind: 'folder' },
    ];
    expect(sortEntries(entries, 'name', false).map((e) => e.name)).toEqual([
      'a',
      'z',
      'f2.log',
      'f10.log',
    ]);
    expect(sortEntries(entries, 'size', true).map((e) => e.name)).toEqual([
      'z',
      'a',
      'f2.log',
      'f10.log',
    ]);
    expect(
      sortEntries(entries, 'modified', false)
        .map((e) => e.name)
        .slice(2),
    ).toEqual(['f2.log', 'f10.log']);
  });

  it('tailles lisibles', () => {
    expect(formatSize(512)).toBe('512 o');
    expect(formatSize(1536)).toBe('1,5 Ko');
    expect(formatSize(5 * 1024 * 1024)).toBe('5 Mo');
    expect(formatSize(undefined)).toBe('');
  });

  it('fil d’Ariane', () => {
    expect(breadcrumbs('logs/2026/')).toEqual([
      { name: 'logs', prefix: 'logs/' },
      { name: '2026', prefix: 'logs/2026/' },
    ]);
    expect(breadcrumbs('')).toEqual([]);
  });

  it('langage de l’aperçu et JSON indenté', () => {
    expect(previewLanguage('a/b.json.gz', '')).toBe('json');
    expect(previewLanguage('conf.yml', '')).toBe('yaml');
    expect(previewLanguage('x.dat', ' <root/>')).toBe('xml');
    expect(previewLanguage('app.log', 'INFO démarrage')).toBe('text');
    expect(prettyText('json', '{"a":1}', false)).toBe('{\n  "a": 1\n}');
    expect(prettyText('json', '{"a":1}\n{"b":2}', false)).toBe('{"a":1}\n{"b":2}');
  });
});
