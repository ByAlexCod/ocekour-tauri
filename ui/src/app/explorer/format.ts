import type { S3Entry } from '../../../shared/api';

export type SortKey = 'name' | 'size' | 'modified';

const collator = new Intl.Collator('fr', { numeric: true, sensitivity: 'base' });

/** Dossiers d'abord, puis tri demandé (nom naturel : « f2 » avant « f10 »). */
export function sortEntries(entries: readonly S3Entry[], key: SortKey, desc: boolean): S3Entry[] {
  const sign = desc ? -1 : 1;
  const compare = (a: S3Entry, b: S3Entry): number => {
    if (a.kind !== b.kind) return a.kind === 'folder' ? -1 : 1;
    let result = 0;
    if (key === 'size') result = (a.size ?? 0) - (b.size ?? 0);
    else if (key === 'modified') result = (a.modified ?? '').localeCompare(b.modified ?? '');
    return sign * (result || collator.compare(a.name, b.name));
  };
  return [...entries].sort(compare);
}

const UNITS = ['o', 'Ko', 'Mo', 'Go', 'To'];

export function formatSize(bytes: number | undefined): string {
  if (bytes === undefined) return '';
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toLocaleString('fr-FR', { maximumFractionDigits: digits })} ${UNITS[unit]}`;
}

/** Fil d'Ariane d'un préfixe : `a/b/` → [a → `a/`, b → `a/b/`]. */
export function breadcrumbs(prefix: string): { name: string; prefix: string }[] {
  const parts = prefix.split('/').filter(Boolean);
  return parts.map((name, i) => ({ name, prefix: parts.slice(0, i + 1).join('/') + '/' }));
}

export type PreviewLanguage = 'json' | 'xml' | 'yaml' | 'text';

/** Coloration de l'aperçu selon l'extension (sans `.gz`) ou le contenu. */
export function previewLanguage(key: string, text: string): PreviewLanguage {
  const ext = key.replace(/\.gz$/i, '').split('.').pop()?.toLowerCase() ?? '';
  if (['json', 'ndjson', 'jsonl', 'geojson'].includes(ext)) return 'json';
  if (['xml', 'xsd', 'xsl', 'svg', 'html', 'htm', 'pom'].includes(ext)) return 'xml';
  if (['yaml', 'yml'].includes(ext)) return 'yaml';
  const start = text.trimStart()[0];
  if (start === '{' || start === '[') return 'json';
  if (start === '<') return 'xml';
  return 'text';
}

/** JSON sur une ligne → indenté (lisible) ; tout le reste est laissé tel quel. */
export function prettyText(language: PreviewLanguage, text: string, truncated: boolean): string {
  if (language !== 'json' || truncated || text.length > 2_000_000) return text;
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text; // NDJSON, ou JSON invalide : tel quel
  }
}

export const s3Uri = (bucket: string, key: string) => `s3://${bucket}/${key}`;
