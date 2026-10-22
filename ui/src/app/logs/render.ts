import type { LogEntry } from './log-buffer';
import { getFieldDeep, type Level } from './parser';

/**
 * Texte d'une cellule affiché au plus (le reste est dans le détail) : au-delà de la largeur
 * d'un écran, il ne ferait que ralentir la mise en page.
 */
export const MAX_ROW_TEXT = 400;

export const LEVEL_CLASS: Record<Level, string> = {
  trace: 'text-slate-400',
  debug: 'text-slate-500 dark:text-slate-400',
  info: 'text-sky-600 dark:text-sky-400',
  warn: 'text-amber-600 dark:text-amber-400',
  error: 'text-red-600 dark:text-red-400',
  fatal: 'text-white bg-red-600 rounded px-0.5 dark:bg-red-700',
  unknown: '',
};

const ESCAPES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

/** Échappe un texte pour l'insérer dans du HTML (contenu ou attribut). */
export const esc = (text: string) => text.replace(/[&<>"']/g, (c) => ESCAPES[c]);

export function fullDate(entry: LogEntry): string {
  return entry.parsed.time?.toLocaleString('fr-FR') ?? '';
}

/** Texte d'une cellule sur une ligne : borné, retours à la ligne visibles (↵). */
export const clip = (text: string) => {
  const short = text.length > MAX_ROW_TEXT ? `${text.slice(0, MAX_ROW_TEXT)}…` : text;
  return short.includes('\n') ? short.replace(/\r?\n/g, ' ↵ ') : short;
};

const timeFormat = new Intl.DateTimeFormat('fr-FR', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });

/** Heure d'une occurrence de notice (ISO → HH:MM:SS.mmm), pour l'affichage déplié. */
export function noticeTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return `${timeFormat.format(date)}.${String(date.getMilliseconds()).padStart(3, '0')}`;
}

/** Heure locale du poste, à la milliseconde. */
export function formatTime(entry: LogEntry): string {
  const date = entry.parsed.time;
  if (!date || Number.isNaN(date.getTime())) return '';
  return `${timeFormat.format(date)}.${String(date.getMilliseconds()).padStart(3, '0')}`;
}

/** Valeur d'une colonne du profil, sur une ligne. */
export function columnText(entry: LogEntry, path: string): string {
  const json = entry.parsed.json;
  if (!json) return '';
  const value = getFieldDeep(json, path);
  if (value === undefined || value === null) return '';
  return typeof value === 'string' ? value : JSON.stringify(value);
}

export function baseClass(entry: LogEntry): string {
  if (entry.notice) return 'bg-sky-50 text-sky-700 dark:bg-sky-950/50 dark:text-sky-300';
  switch (entry.parsed.level) {
    case 'fatal':
    case 'error':
      return 'bg-red-50 hover:bg-red-100 dark:bg-red-950/40 dark:hover:bg-red-950/70';
    case 'warn':
      return 'bg-amber-50/70 hover:bg-amber-100 dark:bg-amber-950/25 dark:hover:bg-amber-950/50';
    default:
      return 'hover:bg-slate-100 dark:hover:bg-slate-800/60';
  }
}

/** Plage de lignes à rendre (zone visible plus une marge), d'après la disposition. */
export function visibleRange(
  layout: { count: number; indexAt(y: number): number },
  scrollTop: number,
  height: number,
  buffer = 600,
): { start: number; end: number } {
  if (!layout.count) return { start: 0, end: 0 };
  const start = layout.indexAt(scrollTop - buffer);
  const end = Math.min(layout.count, layout.indexAt(scrollTop + height + buffer) + 1);
  return { start, end };
}
