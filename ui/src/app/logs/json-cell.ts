import type { LogEntry } from './log-buffer';
import { type FieldNames, getFieldDeep, rawMessage } from './parser';

/** Colonne message (dans `wrap` des profils). */
export const MESSAGE_COLUMN = '@message';
/** Au-delà, la valeur indentée est tronquée (une ligne de log reste lisible). */
export const MAX_WRAP_LINES = 200;

function asJson(value: unknown): object | undefined {
  if (value && typeof value === 'object') return value;
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return undefined;
  try {
    const parsed: unknown = JSON.parse(trimmed);
    return parsed && typeof parsed === 'object' ? parsed : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Valeur JSON d'une colonne, indentée sur plusieurs lignes ; `undefined` si la valeur n'est
 * pas du JSON (objet, tableau, ou chaîne qui en contient) : elle s'affiche alors sur une ligne.
 */
export function wrappedValue(
  entry: LogEntry,
  column: string,
  fields: FieldNames,
): string | undefined {
  if (entry.notice) return undefined;
  const json = entry.parsed.json;
  let value: unknown;
  if (column === MESSAGE_COLUMN) {
    // Message : son champ s'il existe, sinon toute la ligne (valeur Kafka JSON…).
    value = json ? (rawMessage(json, fields) ?? json) : entry.text;
  } else {
    value = json ? getFieldDeep(json, column) : undefined;
  }
  const object = asJson(value);
  if (!object) return undefined;
  const lines = JSON.stringify(object, null, 2).split('\n');
  if (lines.length <= MAX_WRAP_LINES) return lines.join('\n');
  const rest = lines.length - MAX_WRAP_LINES;
  return [...lines.slice(0, MAX_WRAP_LINES), `… (${rest} ligne(s) de plus : voir le détail)`].join(
    '\n',
  );
}

/**
 * Découpe un texte en lignes d'au plus `width` caractères (police à chasse fixe), en coupant de
 * préférence après un espace ; les retours à la ligne du texte sont gardés, les tabulations
 * remplacées par 4 espaces. Au-delà de `maxLines`, la suite est renvoyée au détail.
 */
export function wrapText(text: string, width: number, maxLines = MAX_WRAP_LINES): string {
  const out: string[] = [];
  const max = Math.max(1, Math.floor(width));
  for (const source of text.replace(/\t/g, '    ').split(/\r?\n/)) {
    let line = source;
    while (line.length > max) {
      const space = line.lastIndexOf(' ', max - 1);
      const cut = space >= max / 2 ? space + 1 : max;
      out.push(line.slice(0, cut));
      line = line.slice(cut);
      if (out.length >= maxLines) break;
    }
    if (out.length >= maxLines) break;
    out.push(line);
  }
  if (out.length < maxLines) return out.join('\n');
  // Coupé : signalé seulement s'il restait du texte.
  const full = out.join('').replace(/\n/g, '').length < text.replace(/\t/g, '    ').replace(/\r?\n/g, '').length;
  return full ? [...out.slice(0, maxLines), '… (suite : voir le détail)'].join('\n') : out.join('\n');
}

/**
 * Cellule d'une colonne « à la ligne » (bouton `{}`), découpée à `width` caractères : JSON
 * indenté si la valeur en est, sinon le texte lui-même (logs non JSON compris). `undefined` si
 * la cellule est vide.
 */
export function wrappedCell(
  entry: LogEntry,
  column: string,
  fields: FieldNames,
  width: number,
): string | undefined {
  if (entry.notice) return undefined;
  const json = wrappedValue(entry, column, fields);
  if (json !== undefined) return wrapText(json, width);
  let text: string;
  if (column === MESSAGE_COLUMN) {
    text = entry.parsed.message;
  } else {
    const value = entry.parsed.json ? getFieldDeep(entry.parsed.json, column) : undefined;
    if (value === undefined || value === null) return undefined;
    text = typeof value === 'string' ? value : JSON.stringify(value);
  }
  return text ? wrapText(text, width) : undefined;
}
