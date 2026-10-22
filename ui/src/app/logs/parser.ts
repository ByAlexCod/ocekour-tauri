// Analyse d'une ligne de log : JSON ou texte, heure, niveau, message.
// Aucune dépendance Angular : testable et déplaçable dans un Web Worker si besoin.

export type Level = 'trace' | 'debug' | 'info' | 'warn' | 'error' | 'fatal' | 'unknown';

export const LEVELS: readonly Level[] = ['trace', 'debug', 'info', 'warn', 'error', 'fatal'];

/** Noms des champs clés ; ceux du profil du workload priment sur la détection. */
export interface FieldNames {
  time?: string;
  level?: string;
  message?: string;
}

export interface ParsedLine {
  /** Objet JSON si la ligne entière en est un. */
  json?: Record<string, unknown>;
  time?: Date;
  level: Level;
  /** Message sur une ligne pour la liste. */
  message: string;
}

const CANDIDATES = {
  time: ['@timestamp', 'timestamp', 'time', 'ts', 'date', 'datetime', '@t'],
  level: ['level', 'severity', 'log.level', 'lvl', 'loglevel', 'levelname', '@l', 'log_level'],
  message: ['message', 'msg', '@m', '@mt', 'log', 'text', 'event'],
} as const;

/** Au-delà, la ligne n'est pas analysée comme JSON (coût et mémoire). */
const MAX_JSON = 1_000_000;

// eslint-disable-next-line no-control-regex
const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]/g;

export function stripAnsi(text: string): string {
  return text.includes('\x1b') ? text.replace(ANSI, '') : text;
}

/** Lit `a.b.c` en objet imbriqué, ou la clé plate `"a.b.c"` si elle existe. */
export function getPath(obj: Record<string, unknown>, path: string): unknown {
  if (path in obj) return obj[path];
  // Nom simple absent : rien à parcourir (cas le plus courant, sans découpage).
  if (!path.includes('.')) return undefined;
  let current: unknown = obj;
  for (const key of path.split('.')) {
    if (current === null || typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[key];
  }
  return current;
}

/**
 * Comme `getPath`, mais traverse aussi les chaînes qui contiennent du JSON
 * (`payload.client.id` quand `payload` est une chaîne JSON).
 */
export function getFieldDeep(obj: Record<string, unknown>, path: string): unknown {
  const direct = getPath(obj, path);
  if (direct !== undefined) return direct;
  let current: unknown = obj;
  for (const key of path.split('.')) {
    if (typeof current === 'string') current = expandNested(current, 1);
    if (current === null || typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[key];
  }
  return current;
}

function pick(
  obj: Record<string, unknown>,
  forced: string | undefined,
  candidates: readonly string[],
) {
  if (forced) return getFieldDeep(obj, forced);
  for (const key of candidates) {
    const value = getPath(obj, key);
    if (value !== undefined && value !== null && value !== '') return value;
  }
  return undefined;
}

export function parseLine(text: string, k8sTs?: string, fields: FieldNames = {}): ParsedLine {
  const clean = stripAnsi(text);
  const json = parseJsonObject(clean);
  if (json) {
    const message = pick(json, fields.message, CANDIDATES.message);
    return {
      json,
      time: toDate(pick(json, fields.time, CANDIDATES.time)) ?? toDate(k8sTs),
      level: normalizeLevel(pick(json, fields.level, CANDIDATES.level)),
      message: oneLine(message === undefined ? clean : stringify(message)),
    };
  }
  return { time: toDate(k8sTs), level: textLevel(clean), message: oneLine(clean) };
}

/** Valeur brute du message d'une ligne JSON (champ reconnu ou choisi), sinon `undefined`. */
export function rawMessage(json: Record<string, unknown>, fields: FieldNames = {}): unknown {
  return pick(json, fields.message, CANDIDATES.message);
}

function parseJsonObject(text: string): Record<string, unknown> | undefined {
  // Test rapide sur les caractères extrêmes avant de copier la ligne (trim).
  const first = firstNonSpace(text);
  if (text.charCodeAt(first) !== 123 /* { */ || text.length > MAX_JSON) return undefined;
  const trimmed = first === 0 && text.charCodeAt(text.length - 1) === 125 ? text : text.trim();
  if (!trimmed.endsWith('}')) return undefined;
  try {
    const value = JSON.parse(trimmed);
    return value && typeof value === 'object' && !Array.isArray(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

function firstNonSpace(text: string): number {
  let i = 0;
  while (i < text.length && text.charCodeAt(i) <= 32) i++;
  return i;
}

function stringify(value: unknown): string {
  return typeof value === 'string' ? value : JSON.stringify(value);
}

/** Liste compacte : retours à la ligne visibles, longueur bornée. */
export function oneLine(text: string, max = 2_000): string {
  const short = text.length > max ? text.slice(0, max) + '…' : text;
  if (!short.includes('\n') && !short.includes('\t')) return short;
  return short.replace(/\r?\n/g, ' ⏎ ').replace(/\t/g, ' ');
}

const LEVEL_WORDS: Record<string, Level> = {
  trace: 'trace',
  verbose: 'trace',
  vrb: 'trace',
  finest: 'trace',
  finer: 'trace',
  debug: 'debug',
  dbg: 'debug',
  fine: 'debug',
  d: 'debug',
  info: 'info',
  information: 'info',
  informational: 'info',
  notice: 'info',
  inf: 'info',
  i: 'info',
  warn: 'warn',
  warning: 'warn',
  wrn: 'warn',
  w: 'warn',
  error: 'error',
  err: 'error',
  severe: 'error',
  e: 'error',
  fatal: 'fatal',
  critical: 'fatal',
  crit: 'fatal',
  panic: 'fatal',
  emerg: 'fatal',
  emergency: 'fatal',
  alert: 'fatal',
  ftl: 'fatal',
  f: 'fatal',
};

/** Niveaux texte (WARN, warning, E…) et numériques (pino 10–60, syslog 0–7). */
export function normalizeLevel(value: unknown): Level {
  if (typeof value === 'number') {
    if (value >= 10) {
      if (value >= 60) return 'fatal';
      if (value >= 50) return 'error';
      if (value >= 40) return 'warn';
      if (value >= 30) return 'info';
      if (value >= 20) return 'debug';
      return 'trace';
    }
    // syslog : 0 emerg … 7 debug
    return (
      (['fatal', 'fatal', 'fatal', 'error', 'warn', 'info', 'info', 'debug'] as const)[value] ??
      'unknown'
    );
  }
  if (typeof value !== 'string') return 'unknown';
  const direct = LEVEL_WORDS[value] ?? LEVEL_WORDS[value.toLowerCase()];
  if (direct) return direct;
  const word = value.trim().toLowerCase();
  if (/^\d+$/.test(word)) return normalizeLevel(Number(word));
  return LEVEL_WORDS[word] ?? 'unknown';
}

const TEXT_LEVEL = /\b(TRACE|DEBUG|INFO|NOTICE|WARN(?:ING)?|ERROR|SEVERE|FATAL|CRITICAL|PANIC)\b/;
const LOGFMT_LEVEL = /\b(?:level|lvl|severity)=["']?(\w+)/i;

/** Niveau d'une ligne texte : mot-clé en majuscules ou `level=…` (logfmt). */
function textLevel(text: string): Level {
  const head = text.slice(0, 300);
  const logfmt = LOGFMT_LEVEL.exec(head);
  if (logfmt) return normalizeLevel(logfmt[1]);
  const word = TEXT_LEVEL.exec(head);
  return word ? normalizeLevel(word[1]) : 'unknown';
}

/** ISO 8601, formats Logback (`2026-09-24 10:00:00,123`), epoch s / ms / µs / ns. */
export function toDate(value: unknown): Date | undefined {
  if (value === undefined || value === null || value === '') return undefined;
  if (
    typeof value === 'number' ||
    (typeof value === 'string' && /^\d{9,19}(\.\d+)?$/.test(value))
  ) {
    const n = Number(value);
    const ms = n < 1e11 ? n * 1_000 : n < 1e14 ? n : n < 1e17 ? n / 1_000 : n / 1_000_000;
    const date = new Date(ms);
    return Number.isNaN(date.getTime()) ? undefined : date;
  }
  if (typeof value !== 'string') return undefined;
  // ISO 8601 déjà normalisé (cas courant) : lu directement.
  if (value.charCodeAt(10) === 84 /* T */ && !value.includes(',')) {
    const ms = Date.parse(value);
    if (!Number.isNaN(ms)) return new Date(ms);
  }
  const normalized = value
    .trim()
    .replace(/^(\d{4}-\d\d-\d\d) /, '$1T')
    .replace(/(:\d\d),(\d+)/, '$1.$2');
  const date = new Date(normalized);
  return Number.isNaN(date.getTime()) ? undefined : date;
}

/**
 * Désérialise les chaînes qui contiennent du JSON (`"payload": "{\"a\":1}"`), jusqu'à
 * `maxDepth` niveaux. Renvoie une copie ; l'original n'est pas modifié.
 */
export function expandNested(value: unknown, maxDepth = 3): unknown {
  if (maxDepth <= 0) return value;
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (
      trimmed.length < MAX_JSON &&
      ((trimmed.startsWith('{') && trimmed.endsWith('}')) ||
        (trimmed.startsWith('[') && trimmed.endsWith(']')))
    ) {
      try {
        return expandNested(JSON.parse(trimmed), maxDepth - 1);
      } catch {
        return value;
      }
    }
    return value;
  }
  if (Array.isArray(value)) return value.map((v) => expandNested(v, maxDepth));
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, expandNested(v, maxDepth)]),
    );
  }
  return value;
}
