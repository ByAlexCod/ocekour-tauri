import { computed, signal } from '../core/signals.svelte';
import type {
  OcekourApi,
  S3Entry,
  S3Preview,
  S3Progress,
  S3Ref,
  SourceErrorInfo,
} from '../../../shared/api';
import type { BucketTarget } from '../logs/target';
import { type SortKey, sortEntries } from './format';

/** Opération longue suivie dans l'onglet (téléchargement, envoi, suppression, renommage). */
export interface Operation {
  opId: string;
  label: string;
  progress: S3Progress;
}

/** Onglet explorateur d'un bucket S3 : dossier courant, pages chargées, sélection, aperçu. */
export class ExplorerSession {
  readonly kind = 'explorer' as const;
  readonly ref: S3Ref;

  /** Dossier courant (préfixe, `''` = racine du bucket). */
  readonly prefix = signal('');
  /** Début de nom recherché dans le dossier courant (préfixe S3). */
  readonly search = signal('');
  private readonly loaded = signal<S3Entry[]>([]);
  readonly next = signal<string | undefined>(undefined);
  readonly loading = signal(false);
  readonly error = signal<SourceErrorInfo | undefined>(undefined);
  readonly sort = signal<{ key: SortKey; desc: boolean }>({ key: 'name', desc: false });
  readonly entries = computed(() => sortEntries(this.loaded(), this.sort().key, this.sort().desc));
  readonly selected = signal<ReadonlySet<string>>(new Set());
  readonly selectedEntries = computed(() => {
    const keys = this.selected();
    return this.loaded().filter((e) => keys.has(e.key));
  });

  readonly previewed = signal<S3Entry | undefined>(undefined);
  readonly preview = signal<S3Preview | undefined>(undefined);
  readonly previewError = signal<SourceErrorInfo | undefined>(undefined);
  readonly previewLoading = signal(false);

  readonly operations = signal<Operation[]>([]);

  // Interface commune aux onglets (barre d'onglets, badge d'erreurs).
  readonly status = computed(() => ({
    state: this.error() ? ('error' as const) : ('live' as const),
    message: this.error()?.message,
  }));
  readonly visible = signal(false);
  readonly unseenErrors = signal(0);
  hiddenSince = Date.now();

  private generation = 0;
  private unsubscribe?: () => void;

  constructor(
    readonly target: BucketTarget,
    private readonly api: OcekourApi['s3'] | undefined = window.ocekour?.s3,
  ) {
    this.ref = { connectionId: target.connectionId, bucket: target.bucket };
    this.unsubscribe = this.api?.onProgress((p) => this.onProgress(p));
  }

  /** Recharge le dossier courant (première page). */
  async load() {
    if (!this.api) return;
    const generation = ++this.generation;
    this.loading.set(true);
    this.error.set(undefined);
    const result = await this.api.list(this.ref, this.prefix(), { search: this.search() });
    if (generation !== this.generation) return;
    this.loading.set(false);
    if (result.ok) {
      this.loaded.set(result.value.entries);
      this.next.set(result.value.next);
    } else {
      this.loaded.set([]);
      this.next.set(undefined);
      this.error.set(result.error);
    }
    this.selected.set(new Set());
  }

  /** Page suivante (1 000 entrées de plus). */
  async loadMore() {
    const token = this.next();
    if (!this.api || !token || this.loading()) return;
    const generation = this.generation;
    this.loading.set(true);
    const result = await this.api.list(this.ref, this.prefix(), {
      search: this.search(),
      continuation: token,
    });
    if (generation !== this.generation) return;
    this.loading.set(false);
    if (result.ok) {
      this.loaded.update((list) => [...list, ...result.value.entries]);
      this.next.set(result.value.next);
    } else {
      this.error.set(result.error);
    }
  }

  open(prefix: string) {
    this.prefix.set(prefix);
    this.search.set('');
    this.closePreview();
    void this.load();
  }

  setSearch(text: string) {
    this.search.set(text);
    void this.load();
  }

  setSort(key: SortKey) {
    this.sort.update((s) => ({ key, desc: s.key === key ? !s.desc : false }));
  }

  toggleSelected(key: string) {
    this.selected.update((set) => {
      const next = new Set(set);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  selectAll(all: boolean) {
    this.selected.set(new Set(all ? this.loaded().map((e) => e.key) : []));
  }

  async showPreview(entry: S3Entry) {
    if (!this.api) return;
    this.previewed.set(entry);
    this.preview.set(undefined);
    this.previewError.set(undefined);
    this.previewLoading.set(true);
    const result = await this.api.preview(this.ref, entry.key);
    if (this.previewed() !== entry) return;
    this.previewLoading.set(false);
    if (result.ok) this.preview.set(result.value);
    else this.previewError.set(result.error);
  }

  closePreview() {
    this.previewed.set(undefined);
    this.preview.set(undefined);
    this.previewError.set(undefined);
  }

  count(prefix: string) {
    return this.api!.count(this.ref, prefix);
  }

  /** Lance une opération longue ; `null` si l'utilisateur a annulé la boîte de dialogue. */
  async run(label: string, start: (api: OcekourApi['s3'], ref: S3Ref) => Promise<string | null>) {
    if (!this.api) return;
    const opId = await start(this.api, this.ref);
    if (!opId) return;
    this.operations.update((ops) => [...ops, { opId, label, progress: { opId, done: 0 } }]);
  }

  cancel(opId: string) {
    void this.api?.cancel(opId);
  }

  dismiss(opId: string) {
    this.operations.update((ops) => ops.filter((o) => o.opId !== opId));
  }

  /** Appelé à la fin d'une opération (rafraîchissement, message). */
  onFinished?: (op: Operation) => void;

  private onProgress(progress: S3Progress) {
    const op = this.operations().find((o) => o.opId === progress.opId);
    if (!op) return;
    const updated = { ...op, progress };
    this.operations.update((ops) => ops.map((o) => (o.opId === op.opId ? updated : o)));
    if (progress.finished) {
      this.onFinished?.(updated);
      if (!progress.error) setTimeout(() => this.dismiss(op.opId), 4_000);
    }
  }

  dispose() {
    for (const op of this.operations()) if (!op.progress.finished) this.cancel(op.opId);
    this.unsubscribe?.();
  }
}
