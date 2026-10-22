import type { KafkaMeta, LogLine } from '../../../shared/api';
import { type FieldNames, type Level, LEVELS, type ParsedLine, parseLine } from './parser';

/** Source d'une ligne dans une vue d'assemblage (membre d'origine). */
export interface LineSource {
  id: string;
  label: string;
  color: string;
}

export interface LogEntry {
  /** Numéro croissant, stable : sert d'identifiant de ligne. */
  seq: number;
  ts?: string;
  text: string;
  parsed: ParsedLine;
  /** Message Kafka : partition, offset, clé, headers… */
  meta?: KafkaMeta;
  /** Texte parcouru par la recherche : la ligne, plus la clé et les headers Kafka. */
  haystack: string;
  /** Événement inséré par Ocekour (redémarrage, reconnexion…) et non ligne de log. */
  notice?: boolean;
  /** Notice répétée : nombre d'occurrences consécutives identiques regroupées (≥ 1). */
  repeat?: number;
  /** Horodatages (ISO) des occurrences regroupées, pour l'affichage détaillé (borné). */
  times?: readonly string[];
  /** Assemblage : membre d'origine de la ligne (badge, filtre par source). */
  source?: LineSource;
}

export type LevelCounts = Record<Level | 'unknown', number>;

/** Horodatages gardés au plus pour une notice regroupée (le compteur, lui, n'est pas borné). */
const MAX_NOTICE_TIMES = 1_000;

const emptyCounts = (): LevelCounts =>
  Object.fromEntries([...LEVELS, 'unknown'].map((l) => [l, 0])) as LevelCounts;

/**
 * Tampon des lignes d'un flux, analysées à l'arrivée : au-delà de `capacity`, les plus
 * anciennes sont retirées (compteur `dropped`). Retrait en O(1) (index de début, tableau
 * compacté de temps en temps) et nombre de lignes par niveau tenu à jour au fil de l'eau :
 * aucun parcours complet à chaque lot reçu.
 */
export class LogBuffer {
  private items: LogEntry[] = [];
  /** Index de la première ligne gardée dans `items`. */
  private head = 0;
  private nextSeq = 0;
  private fields: FieldNames = {};
  private fieldsKey = '{}';
  private counts = emptyCounts();
  dropped = 0;

  constructor(private capacity: number) {}

  /** Lignes gardées, de la plus ancienne à la plus récente (copie évitée si possible). */
  get entries(): readonly LogEntry[] {
    this.compact();
    return this.items;
  }

  get length(): number {
    return this.items.length - this.head;
  }

  at(index: number): LogEntry | undefined {
    return this.items[this.head + index];
  }

  /** Numéro de la plus ancienne ligne gardée (lignes plus anciennes : retirées). */
  get firstSeq(): number {
    return this.items[this.head]?.seq ?? this.nextSeq;
  }

  /** Nombre total de lignes reçues (y compris celles retirées). */
  get total(): number {
    return this.nextSeq;
  }

  /** Nombre de lignes par niveau (hors événements Ocekour). */
  get levelCounts(): Readonly<LevelCounts> {
    return this.counts;
  }

  /** Ajoute des lignes ; renvoie les entrées créées. */
  push(lines: readonly LogLine[]): LogEntry[] {
    const added: LogEntry[] = new Array(lines.length);
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const entry: LogEntry = {
        seq: this.nextSeq++,
        ts: line.ts,
        text: line.text,
        parsed: parseLine(line.text, line.ts, this.fields),
        meta: line.meta,
        haystack: line.meta ? haystack(line.text, line.meta) : line.text,
      };
      this.counts[entry.parsed.level]++;
      this.items.push(entry);
      added[i] = entry;
    }
    this.trim();
    return added;
  }

  /**
   * Ajoute une ligne étiquetée d'une source, analysée avec les champs donnés (assemblage :
   * chaque membre garde son profil). Renvoie l'entrée créée.
   */
  pushTagged(line: LogLine, fields: FieldNames, source: LineSource): LogEntry {
    const entry: LogEntry = {
      seq: this.nextSeq++,
      ts: line.ts,
      text: line.text,
      parsed: parseLine(line.text, line.ts, fields),
      meta: line.meta,
      haystack: line.meta ? haystack(line.text, line.meta) : line.text,
      source,
    };
    this.counts[entry.parsed.level]++;
    this.items.push(entry);
    this.trim();
    return entry;
  }

  /**
   * Insère un événement Ocekour (redémarrage, flux interrompu…). Si la dernière ligne est déjà
   * une notice de même texte, elle n'est pas dupliquée : son compteur est incrémenté (et l'heure
   * mise à jour). `created` vaut `false` dans ce cas (la ligne existante a été mise à jour).
   */
  notice(text: string): { entry: LogEntry; created: boolean } {
    const ts = new Date().toISOString();
    const last = this.items[this.items.length - 1];
    if (last?.notice && last.text === text) {
      last.repeat = (last.repeat ?? 1) + 1;
      last.ts = ts;
      last.parsed = { level: 'unknown', message: text, time: new Date(ts) };
      const times = last.times as string[] | undefined;
      if (times && times.length < MAX_NOTICE_TIMES) times.push(ts);
      return { entry: last, created: false };
    }
    const entry: LogEntry = {
      seq: this.nextSeq++,
      ts,
      text,
      haystack: text,
      notice: true,
      repeat: 1,
      times: [ts],
      parsed: { level: 'unknown', message: text, time: new Date(ts) },
    };
    this.items.push(entry);
    this.trim();
    return { entry, created: true };
  }

  /**
   * Change les champs clés (profil) et ré-analyse les lignes déjà reçues ; sans effet si
   * les champs sont les mêmes. Renvoie `true` si les lignes ont été ré-analysées.
   */
  setFields(fields: FieldNames): boolean {
    const key = JSON.stringify(fields);
    if (key === this.fieldsKey) return false;
    this.fields = fields;
    this.fieldsKey = key;
    this.counts = emptyCounts();
    for (let i = this.head; i < this.items.length; i++) {
      const entry = this.items[i];
      if (entry.notice) continue;
      entry.parsed = parseLine(entry.text, entry.ts, fields);
      this.counts[entry.parsed.level]++;
    }
    return true;
  }

  /**
   * Ré-ordonne les lignes par horodatage (assemblage : fusion multi-sources). Les numéros de
   * ligne sont réattribués pour rester croissants dans le nouvel ordre (recherche par numéro,
   * liste virtuelle). À appeler ponctuellement (ajout d'une source, ligne très en retard).
   */
  reorderByTime() {
    this.compact();
    this.items.sort((a, b) => timeKey(a) - timeKey(b) || a.seq - b.seq);
    for (let i = 0; i < this.items.length; i++) this.items[i].seq = i;
    this.nextSeq = this.items.length;
  }

  clear() {
    this.items = [];
    this.head = 0;
    this.counts = emptyCounts();
    this.dropped = 0;
  }

  setCapacity(capacity: number) {
    this.capacity = capacity;
    this.trim();
  }

  private trim() {
    const excess = this.length - this.capacity;
    if (excess <= 0) return;
    for (let i = this.head; i < this.head + excess; i++) {
      const entry = this.items[i];
      if (!entry.notice) this.counts[entry.parsed.level]--;
    }
    this.head += excess;
    this.dropped += excess;
    // Tableau compacté quand la moitié est inutilisée : coût amorti constant.
    if (this.head > 4_096 && this.head * 2 > this.items.length) this.compact();
  }

  private compact() {
    if (!this.head) return;
    this.items = this.items.slice(this.head);
    this.head = 0;
  }
}

/** Clé de tri temporel d'une ligne : horodatage analysé, horodatage brut, sinon fin de liste. */
export function timeKey(entry: LogEntry): number {
  const t = entry.parsed.time?.getTime();
  if (t !== undefined && !Number.isNaN(t)) return t;
  const raw = entry.ts ? Date.parse(entry.ts) : NaN;
  return Number.isNaN(raw) ? Number.MAX_SAFE_INTEGER : raw;
}

function haystack(text: string, meta: KafkaMeta): string {
  const headers = Object.entries(meta.headers ?? {}).map(([k, v]) => `${k}=${v}`);
  return [text, meta.key ?? '', ...headers].join('\n');
}
