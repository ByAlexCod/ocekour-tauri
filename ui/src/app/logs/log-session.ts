import { signal } from '../core/signals.svelte';
import type {
  SourceErrorInfo,
  SourcePath,
  StreamEvent,
  StreamOptions,
  StreamState,
} from '../../../shared/api';
import { type StreamTarget, streamOptions, streamPath } from './target';
import type { SearchOptions } from './filter';
import { LogBuffer } from './log-buffer';
import type { LogFilterRule } from './log-filters';
import { type RowFilter, LogRows } from './log-rows';
import { type Level, LEVELS } from './parser';

/** Hauteur d'une ligne de la liste (px). */
export const ROW_HEIGHT = 20;

export interface SessionStatus {
  state: StreamState;
  message?: string;
  error?: SourceErrorInfo;
  replacements?: string[];
}

/**
 * Un flux de logs affiché : options, tampon de lignes, état. Rouvre le flux côté main
 * quand les options changent.
 */
export class LogSession {
  readonly kind = 'stream' as const;
  readonly buffer: LogBuffer;
  /** Lignes affichées (filtres appliqués), tenues à jour à chaque lot. */
  readonly rows = new LogRows(ROW_HEIGHT);
  readonly options = signal<StreamOptions>({ follow: true });
  readonly status = signal<SessionStatus>({ state: 'connecting' });
  /**
   * Incrémenté quand l'affichage doit être mis à jour (tampon et lignes sont mutés sur
   * place) : au plus une fois par image, quel que soit le débit du flux.
   */
  readonly version = signal(0);
  private frameRequested = false;
  /** Recherche texte / regex. */
  readonly search = signal<SearchOptions>({ text: '', caseSensitive: false, regex: false });
  /** Niveaux affichés (`unknown` = lignes sans niveau reconnu). */
  readonly levels = signal<ReadonlySet<Level>>(new Set([...LEVELS, 'unknown']));
  /** « filtrer » masque les lignes qui ne correspondent pas ; « surligner » les garde. */
  readonly searchMode = signal<'filter' | 'highlight'>('filter');
  /** Règles de filtre (masquer / garder seul) par champ. */
  readonly rules = signal<LogFilterRule[]>([]);
  // État d'affichage, conservé quand on passe d'un onglet à l'autre.
  /** Affichage figé : les lignes continuent d'arriver dans le tampon. */
  readonly paused = signal(false);
  /** Colle en bas de la liste tant que l'utilisateur ne remonte pas. */
  readonly stickToBottom = signal(true);
  readonly seqWhenUnstuck = signal(0);
  readonly selectedSeq = signal<number | null>(null);
  scrollTop = 0;
  /** Filtre appliqué aux lignes affichées (évite de refiltrer au retour sur l'onglet). */
  filterKey?: string;
  filterText?: string;
  /** Réglage des hauteurs de ligne appliqué (colonnes JSON indentées). */
  heightsKey?: string;
  /** Onglet affiché ; sinon les nouvelles erreurs sont comptées. */
  readonly visible = signal(false);
  readonly unseenErrors = signal(0);
  /** Depuis quand l'onglet est caché : seules les erreurs postérieures sont « nouvelles ». */
  hiddenSince = Date.now();
  /** Lignes reçues pendant la dernière seconde. */
  readonly rate = signal(0);
  streamId?: string;
  private lastTotal = 0;
  private readonly rateTimer = setInterval(() => {
    this.rate.set(this.buffer.total - this.lastTotal);
    this.lastTotal = this.buffer.total;
  }, 1_000);

  constructor(
    readonly target: StreamTarget,
    options: StreamOptions,
    bufferSize: number,
    private readonly api: NonNullable<Window['ocekour']>['streams'],
    private readonly register: (id: string, session: LogSession) => void,
  ) {
    this.buffer = new LogBuffer(bufferSize);
    this.options.set(options);
  }

  get sourcePath(): SourcePath {
    return streamPath(this.target);
  }

  /** Demande une mise à jour de l'affichage (regroupée à l'image suivante). */
  invalidate() {
    if (this.frameRequested) return;
    this.frameRequested = true;
    const run = () => {
      this.frameRequested = false;
      this.version.update((v) => v + 1);
    };
    // Fenêtre cachée : pas d'image, la mise à jour attend son retour (rAF suspendu).
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run);
    else queueMicrotask(run);
  }

  /** Filtres changés : lignes affichées recalculées. */
  setFilter(filter: RowFilter) {
    this.rows.reset(this.buffer, filter);
    this.invalidate();
  }

  /** Recalcule les lignes affichées (champs clés ou pause changés). */
  refresh() {
    this.rows.reset(this.buffer);
    this.invalidate();
  }

  async start() {
    await this.stop();
    this.buffer.clear();
    this.rows.reset(this.buffer);
    this.invalidate();
    this.status.set({ state: 'connecting' });
    try {
      this.streamId = await this.api.open(this.sourcePath, streamOptions(this.target, this.options()));
      this.register(this.streamId, this);
    } catch (e) {
      this.status.set({ state: 'error', message: (e as Error).message });
    }
  }

  /** Change les options et relance le flux. */
  restart(patch: Partial<StreamOptions>) {
    this.options.update((o) => ({ ...o, ...patch }));
    return this.start();
  }

  /** Session fermée : flux arrêté, minuterie libérée. */
  dispose() {
    clearInterval(this.rateTimer);
    return this.stop();
  }

  async stop() {
    if (!this.streamId) return;
    const id = this.streamId;
    this.streamId = undefined;
    await this.api.close(id);
  }

  handle(event: StreamEvent, _streamId?: string) {
    switch (event.type) {
      case 'lines': {
        const added = this.buffer.push(event.lines);
        // En pause, l'affichage reste figé : il est recalculé à la reprise.
        if (!this.paused()) {
          this.rows.append(added);
          this.rows.trimBefore(this.buffer.firstSeq);
        }
        if (!this.visible()) {
          // L'historique chargé à l'ouverture n'est pas « nouveau ».
          const errors = added.filter(
            (e) =>
              (e.parsed.level === 'error' || e.parsed.level === 'fatal') &&
              (e.parsed.time?.getTime() ?? Date.now()) >= this.hiddenSince,
          );
          if (errors.length) this.unseenErrors.update((n) => n + errors.length);
        }
        break;
      }
      case 'notice':
        this.notice(event.message);
        break;
      case 'status': {
        const previous = this.status().state;
        if (event.state === 'reconnecting' && previous === 'live') {
          this.notice(event.message ?? 'Flux interrompu');
          this.invalidate();
        }
        this.status.set(event);
        return;
      }
    }
    this.invalidate();
  }

  private notice(message: string) {
    const { entry, created } = this.buffer.notice(message);
    if (this.paused()) return;
    if (created) {
      this.rows.append([entry]);
      this.rows.trimBefore(this.buffer.firstSeq);
    } else {
      // Notice regroupée (même texte) : le compteur a changé, on rafraîchit l'affichage.
      this.invalidate();
    }
  }
}
