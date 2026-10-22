import { signal } from '../core/signals.svelte';
import type { SourceErrorInfo, StreamEvent, StreamOptions, StreamState } from '../../../shared/api';
import type { SearchOptions } from './filter';
import type { LogFilterRule } from './log-filters';
import { type LineSource, LogBuffer, timeKey } from './log-buffer';
import { type RowFilter, LogRows } from './log-rows';
import { ROW_HEIGHT } from './log-session';
import { type FieldNames, type Level, LEVELS } from './parser';
import {
  type AssemblyMember,
  type AssemblyTarget,
  type StreamTarget,
  streamOptions,
  streamPath,
} from './target';

/** Interface des flux Ocekour (preload). */
type StreamsApi = NonNullable<Window['ocekour']>['streams'];

/** Lie / délie un identifiant de flux à un destinataire d'événements (StreamService). */
export interface StreamBinder {
  api: StreamsApi;
  bind(id: string, session: AssemblySession): void;
  unbind(id: string): void;
}

/** Champs clés (profil) d'une cible de flux. */
export type FieldsResolver = (target: StreamTarget) => FieldNames;

/** Fenêtre de réordonnancement en direct par défaut (ms) ; 0 = ordre d'arrivée. */
export const DEFAULT_REORDER_MS = 2_000;
/** Cadence de vidage du tampon de réordonnancement. */
const FLUSH_MS = 250;

interface LiveMember extends AssemblyMember {
  fields: FieldNames;
  source: LineSource;
  streamId?: string;
  state: StreamState;
}

interface Pending {
  line: { ts?: string; text: string; meta?: import('../../../shared/api').KafkaMeta };
  member: LiveMember;
  recvAt: number;
  /** Clé de tri : horodatage de la ligne, ou heure de réception si absente. */
  sortKey: number;
}

/**
 * Assemblage : plusieurs flux (conteneurs, topics) fusionnés dans une vue triée dans le temps.
 * Même surface d'affichage qu'une `LogSession` (tampon, lignes, filtres, pause…), plus la gestion
 * des membres et de la fenêtre de réordonnancement. Composition côté front : réutilise `streams.open`.
 */
export class AssemblySession {
  readonly kind = 'assembly' as const;
  readonly buffer: LogBuffer;
  readonly rows = new LogRows(ROW_HEIGHT);
  readonly options = signal<StreamOptions>({ follow: true });
  readonly status = signal<{
    state: StreamState;
    message?: string;
    error?: SourceErrorInfo;
    replacements?: string[];
  }>({ state: 'connecting' });
  readonly version = signal(0);
  private frameRequested = false;

  readonly search = signal<SearchOptions>({ text: '', caseSensitive: false, regex: false });
  readonly levels = signal<ReadonlySet<Level>>(new Set([...LEVELS, 'unknown']));
  readonly searchMode = signal<'filter' | 'highlight'>('filter');
  readonly rules = signal<LogFilterRule[]>([]);
  readonly paused = signal(false);
  readonly stickToBottom = signal(true);
  readonly seqWhenUnstuck = signal(0);
  readonly selectedSeq = signal<number | null>(null);
  scrollTop = 0;
  filterKey?: string;
  filterText?: string;
  heightsKey?: string;
  readonly visible = signal(false);
  readonly unseenErrors = signal(0);
  hiddenSince = Date.now();
  readonly rate = signal(0);

  /** Membres (réactif : badges, filtre par source). */
  readonly members = signal<LiveMember[]>([]);
  /** Sources masquées (filtre par source). */
  readonly hiddenSources = signal<ReadonlySet<string>>(new Set());
  /** Fenêtre de réordonnancement en direct (ms) ; 0 = ordre d'arrivée. */
  readonly reorderMs = signal(DEFAULT_REORDER_MS);

  private readonly pending: Pending[] = [];
  /** Plus grand horodatage déjà inséré : une ligne plus ancienne déclenche un ré-ordonnancement. */
  private maxKey = -Infinity;
  private dirtyOrder = false;
  private lastTotal = 0;
  private readonly rateTimer = setInterval(() => {
    this.rate.set(this.buffer.total - this.lastTotal);
    this.lastTotal = this.buffer.total;
  }, 1_000);
  private flushTimer?: ReturnType<typeof setInterval>;

  constructor(
    private target_: AssemblyTarget,
    options: StreamOptions,
    bufferSize: number,
    private readonly binder: StreamBinder,
    private readonly resolveFields: FieldsResolver,
  ) {
    this.buffer = new LogBuffer(bufferSize);
    this.options.set(options);
    this.members.set(target_.members.map((m, i) => this.toLive(m, i)));
  }

  /** Cible de l'onglet (reconstruite depuis les membres courants). */
  get target(): AssemblyTarget {
    return { ...this.target_, members: this.members().map(toMember) };
  }

  private toLive(m: AssemblyMember, _index: number): LiveMember {
    return {
      ...m,
      fields: this.resolveFields(m.target),
      source: { id: m.id, label: m.label, color: m.color },
      state: 'connecting',
    };
  }

  invalidate() {
    if (this.frameRequested) return;
    this.frameRequested = true;
    const run = () => {
      this.frameRequested = false;
      this.version.update((v) => v + 1);
    };
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run);
    else queueMicrotask(run);
  }

  /** Dernier filtre d'affichage (hors sources masquées), pour le réappliquer. */
  private lastFilter: RowFilter = {};

  /** Filtre d'affichage : on ajoute toujours les sources masquées de l'assemblage. */
  setFilter(filter: RowFilter) {
    this.lastFilter = filter;
    this.rows.reset(this.buffer, { ...filter, hiddenSources: this.hiddenSources() });
    this.invalidate();
  }

  refresh() {
    this.rows.reset(this.buffer, { ...this.lastFilter, hiddenSources: this.hiddenSources() });
    this.invalidate();
  }

  /** Masque / affiche une source, sans fermer son flux. */
  toggleSource(id: string) {
    const next = new Set(this.hiddenSources());
    next.has(id) ? next.delete(id) : next.add(id);
    this.hiddenSources.set(next);
    this.refresh();
  }

  rename(name: string) {
    this.target_ = { ...this.target_, name: name.trim() || undefined };
  }

  async start() {
    await this.stop();
    this.buffer.clear();
    this.pending.length = 0;
    this.maxKey = -Infinity;
    this.dirtyOrder = false;
    this.rows.reset(this.buffer, { hiddenSources: this.hiddenSources() });
    this.invalidate();
    this.status.set({ state: 'connecting' });
    this.flushTimer ??= setInterval(() => this.flush(), FLUSH_MS);
    await Promise.all(this.members().map((m) => this.openMember(m)));
    this.aggregateStatus();
  }

  restart(patch: Partial<StreamOptions>) {
    this.options.update((o) => ({ ...o, ...patch }));
    return this.start();
  }

  /** Ajoute une source (dans la limite). Renvoie `false` si la limite est atteinte. */
  async addMember(member: AssemblyMember): Promise<boolean> {
    const live = this.toLive(member, this.members().length);
    this.members.update((ms) => [...ms, live]);
    await this.openMember(live);
    this.aggregateStatus();
    return true;
  }

  /** Retire une source : ferme son flux et retire ses lignes de l'affichage. */
  removeMember(id: string) {
    const member = this.members().find((m) => m.id === id);
    if (member?.streamId) this.binder.unbind(member.streamId);
    this.members.update((ms) => ms.filter((m) => m.id !== id));
    // Les lignes déjà reçues restent dans le tampon ; on les masque via le filtre.
    const next = new Set(this.hiddenSources());
    next.add(id);
    this.hiddenSources.set(next);
    this.refresh();
  }

  private async openMember(member: LiveMember) {
    if (member.streamId) this.binder.unbind(member.streamId);
    member.state = 'connecting';
    try {
      const id = await this.binder.api.open(streamPath(member.target), streamOptions(member.target, this.options()));
      member.streamId = id;
      this.binder.bind(id, this);
    } catch (e) {
      member.state = 'error';
      this.notice(`${member.label} : ${(e as Error).message}`);
    }
  }

  dispose() {
    clearInterval(this.rateTimer);
    clearInterval(this.flushTimer);
    this.flushTimer = undefined;
    return this.stop();
  }

  async stop() {
    const ids = this.members()
      .map((m) => m.streamId)
      .filter((id): id is string => !!id);
    for (const m of this.members()) m.streamId = undefined;
    for (const id of ids) this.binder.unbind(id);
    await Promise.all(ids.map((id) => this.binder.api.close(id).catch(() => undefined)));
  }

  /** Événement d'un flux membre (le `StreamService` passe l'identifiant). */
  handle(event: StreamEvent, streamId?: string) {
    const member = this.members().find((m) => m.streamId === streamId);
    if (!member) return;
    switch (event.type) {
      case 'lines': {
        const now = Date.now();
        if (this.reorderMs() <= 0) {
          for (const line of event.lines) this.insert(line, member);
        } else {
          for (const line of event.lines) {
            this.pending.push({ line, member, recvAt: now, sortKey: sortKey(line.ts, now) });
          }
        }
        break;
      }
      case 'notice':
        this.notice(`${member.label} : ${event.message}`);
        break;
      case 'status': {
        if (event.state === 'reconnecting' && member.state === 'live') {
          this.notice(`${member.label} : ${event.message ?? 'flux interrompu'}`);
        }
        member.state = event.state;
        this.aggregateStatus();
        return;
      }
    }
    this.invalidate();
  }

  /** Vide le tampon de réordonnancement : lignes « dues » insérées triées par le temps. */
  private flush() {
    if (!this.pending.length) return;
    const cutoff = Date.now() - this.reorderMs();
    const due: Pending[] = [];
    for (let i = this.pending.length - 1; i >= 0; i--) {
      if (this.pending[i].recvAt <= cutoff) due.push(...this.pending.splice(i, 1));
    }
    if (!due.length) return;
    due.sort((a, b) => a.sortKey - b.sortKey);
    for (const p of due) this.insert(p.line, p.member);
    this.finishBatch();
  }

  /**
   * Fin d'un lot : si une ligne plus ancienne que le reste a été insérée (source ajoutée, ligne
   * en retard), on ré-ordonne tout le tampon par horodatage et on reconstruit l'affichage.
   */
  private finishBatch() {
    if (this.dirtyOrder && this.reorderMs() > 0) {
      this.buffer.reorderByTime();
      this.rows.reset(this.buffer, { ...this.lastFilter, hiddenSources: this.hiddenSources() });
      const entries = this.buffer.entries;
      this.maxKey = entries.length ? timeKey(entries[entries.length - 1]) : -Infinity;
      this.dirtyOrder = false;
    }
    this.invalidate();
  }

  /** Insère une ligne étiquetée dans le tampon et l'affichage. */
  private insert(line: Pending['line'], member: LiveMember) {
    const entry = this.buffer.pushTagged(line, member.fields, member.source);
    const key = timeKey(entry);
    if (key < this.maxKey) this.dirtyOrder = true;
    else this.maxKey = key;
    if (!this.paused()) {
      this.rows.append([entry]);
      this.rows.trimBefore(this.buffer.firstSeq);
    }
    if (!this.visible() && (entry.parsed.level === 'error' || entry.parsed.level === 'fatal')) {
      const at = entry.parsed.time?.getTime() ?? Date.now();
      if (at >= this.hiddenSince) this.unseenErrors.update((n) => n + 1);
    }
  }

  private notice(message: string) {
    const { entry, created } = this.buffer.notice(message);
    if (this.paused()) return;
    if (created) {
      this.rows.append([entry]);
      this.rows.trimBefore(this.buffer.firstSeq);
    } else {
      this.invalidate();
    }
  }

  /** État global de l'assemblage : le plus sévère parmi les membres. */
  private aggregateStatus() {
    const states = this.members().map((m) => m.state);
    if (!states.length) return this.status.set({ state: 'ended' });
    const errored = this.members().filter((m) => m.state === 'error');
    if (errored.length) {
      this.status.set({
        state: 'error',
        message: `${errored.length} source(s) en erreur : ${errored.map((m) => m.label).join(', ')}`,
      });
    } else if (states.some((s) => s === 'reconnecting')) {
      this.status.set({ state: 'reconnecting', message: 'Reconnexion…' });
    } else if (states.some((s) => s === 'connecting')) {
      this.status.set({ state: 'connecting' });
    } else {
      this.status.set({ state: 'live' });
    }
  }
}

function toMember(m: LiveMember): AssemblyMember {
  return { id: m.id, target: m.target, label: m.label, color: m.color };
}

function sortKey(ts: string | undefined, now: number): number {
  const t = ts ? Date.parse(ts) : NaN;
  return Number.isNaN(t) ? now : t;
}
