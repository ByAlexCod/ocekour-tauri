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
