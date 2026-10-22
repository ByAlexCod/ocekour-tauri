import type { Matcher } from './filter';
import type { LogBuffer, LogEntry } from './log-buffer';
import { type CompiledRule, rulesAccept } from './log-filters';
import type { Level } from './parser';
import type { RowGeometry } from './row-layout';

/** Critères d'affichage : niveaux cochés, recherche en mode « filtrer », surlignage. */
export interface RowFilter {
  /** Niveaux affichés ; `undefined` = tous. */
  levels?: ReadonlySet<Level>;
  /** Recherche en mode « filtrer » : lignes masquées si elles ne correspondent pas. */
  matcher?: Matcher;
  /** Recherche en mode « surligner » : correspondances comptées. */
  highlight?: Matcher;
  /** Assemblage : ids de sources masquées (lignes exclues de l'affichage). */
  hiddenSources?: ReadonlySet<string>;
  /** Règles de filtre (masquer / garder seul) par champ. */
  rules?: readonly CompiledRule[];
}

/**
 * Lignes affichées d'un flux (filtres appliqués), tenues à jour **incrémentalement** : un lot
 * reçu est filtré seul, les lignes retirées du tampon sont retirées en tête (O(1) amorti).
 * Sert aussi de disposition (hauteur de chaque ligne) à la liste virtuelle, et de source de
 * données : `length` et `slice` suffisent à `cdkVirtualFor`, qui ne lit que la tranche visible.
 */
export class LogRows implements RowGeometry {
  private items: LogEntry[] = [];
  private head = 0;
  private filter: RowFilter = {};
  /** Hauteur variable d'une ligne (JSON indenté) ; `undefined` = toutes à `baseHeight`. */
  private heights?: (entry: LogEntry) => number;
  /** Positions cumulées (`offsets[k]` = haut de `items[k]`), si hauteurs variables. */
  private offsets = new Float64Array(1);
  /** Correspondances de la recherche en mode « surligner ». */
  matchCount = 0;

  constructor(readonly baseHeight: number) {}

  // --- Données (liste virtuelle) ---------------------------------------------------------

  get length(): number {
    return this.items.length - this.head;
  }

  get count(): number {
    return this.length;
  }

  at(index: number): LogEntry | undefined {
    return index >= 0 && index < this.length ? this.items[this.head + index] : undefined;
  }

  slice(start = 0, end = this.length): LogEntry[] {
    return this.items.slice(this.head + Math.max(0, start), this.head + Math.min(end, this.length));
  }

  toArray(): LogEntry[] {
    return this.slice();
  }

  /** Index d'une ligne par son numéro (recherche dichotomique : les numéros sont croissants). */
  indexOfSeq(seq: number | null | undefined): number {
    if (seq === null || seq === undefined) return -1;
    let low = this.head;
    let high = this.items.length - 1;
    while (low <= high) {
      const mid = (low + high) >> 1;
      const value = this.items[mid].seq;
      if (value === seq) return mid - this.head;
      if (value < seq) low = mid + 1;
      else high = mid - 1;
    }
    return -1;
  }

  /** Prochaine ligne (à partir de `from`, dans le sens `direction`) qui vérifie `test`. */
  find(from: number, direction: 1 | -1, test: (e: LogEntry) => boolean): number {
    const n = this.length;
    for (let k = 1; k <= n; k++) {
      const i = (((from + direction * k) % n) + n) % n;
      if (test(this.items[this.head + i])) return i;
    }
    return -1;
  }

  // --- Mise à jour -----------------------------------------------------------------------

  private accepts(entry: LogEntry): boolean {
    if (entry.notice) return true;
    const { levels, matcher, hiddenSources, rules } = this.filter;
    if (hiddenSources?.size && entry.source && hiddenSources.has(entry.source.id)) return false;
    if (levels && !levels.has(entry.parsed.level)) return false;
    if (matcher && !matcher.test(entry.haystack)) return false;
    if (rules?.length && !rulesAccept(entry, rules)) return false;
    return true;
  }

  private isMatch(entry: LogEntry): boolean {
    return !entry.notice && !!this.filter.highlight?.test(entry.haystack);
  }

  /** Recalcule tout (filtre, hauteurs ou champs changés). */
  reset(buffer: LogBuffer, filter: RowFilter = this.filter) {
    this.filter = filter;
    this.items = [];
    this.head = 0;
    this.matchCount = 0;
    this.offsets = new Float64Array(Math.max(1_024, buffer.length + 1));
    this.appendAll(buffer.entries);
  }

  /** Ajoute les lignes d'un lot qui passent le filtre. */
  append(entries: readonly LogEntry[]) {
    this.appendAll(entries);
  }

  private appendAll(entries: readonly LogEntry[]) {
    const heights = this.heights;
    for (const entry of entries) {
      if (!this.accepts(entry)) continue;
      if (this.filter.highlight && this.isMatch(entry)) this.matchCount++;
      if (heights) {
        const k = this.items.length;
        if (k + 2 > this.offsets.length) {
          const grown = new Float64Array(this.offsets.length * 2);
          grown.set(this.offsets);
          this.offsets = grown;
        }
        this.offsets[k + 1] = this.offsets[k] + heights(entry);
      }
      this.items.push(entry);
    }
  }

  /** Retire en tête les lignes plus anciennes que `seq` (retirées du tampon). */
  trimBefore(seq: number) {
    const start = this.head;
    while (this.head < this.items.length && this.items[this.head].seq < seq) {
      if (this.filter.highlight && this.isMatch(this.items[this.head])) this.matchCount--;
      this.head++;
    }
    if (this.head !== start && this.head > 4_096 && this.head * 2 > this.items.length) {
      this.compact();
    }
  }

  private compact() {
    if (this.heights) {
      const base = this.offsets[this.head];
      const size = this.items.length - this.head;
      const next = new Float64Array(Math.max(1_024, size * 2));
      for (let k = 0; k <= size; k++) next[k] = this.offsets[this.head + k] - base;
      this.offsets = next;
    }
    this.items = this.items.slice(this.head);
    this.head = 0;
  }

  /** Change la hauteur des lignes (JSON indenté) ; recalcule si besoin. */
  setHeights(heights: ((entry: LogEntry) => number) | undefined, buffer: LogBuffer) {
    if (heights === this.heights) return;
    this.heights = heights;
    this.reset(buffer);
  }

  // --- Disposition (hauteurs variables) ----------------------------------------------------

  get total(): number {
    return this.heights
      ? this.offsets[this.items.length] - this.offsets[this.head]
      : this.length * this.baseHeight;
  }

  offsetOf(index: number): number {
    const i = Math.min(Math.max(index, 0), this.length);
    return this.heights
      ? this.offsets[this.head + i] - this.offsets[this.head]
      : i * this.baseHeight;
  }

  heightOf(index: number): number {
    if (!this.heights) return this.baseHeight;
    const k = this.head + index;
    return this.offsets[k + 1] - this.offsets[k];
  }

  /** Ligne sous la position `y` (bornée à la liste). */
  indexAt(y: number): number {
    const n = this.length;
    if (n === 0) return 0;
    if (!this.heights) return Math.min(Math.max(Math.floor(y / this.baseHeight), 0), n - 1);
    const target = y + this.offsets[this.head];
    let low = this.head;
    let high = this.items.length - 1;
    while (low < high) {
      const mid = (low + high + 1) >> 1;
      if (this.offsets[mid] <= target) low = mid;
      else high = mid - 1;
    }
    return low - this.head;
  }
}
