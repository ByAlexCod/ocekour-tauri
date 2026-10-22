<script lang="ts">
  import { untrack } from 'svelte';
  import type { S3Entry } from '../../../shared/api';
  import { analytics } from '../core/analytics';
  import { copy } from '../core/clipboard';
  import { notifications } from '../core/notification.svelte';
  import { RowLayout } from '../logs/row-layout';
  import { visibleRange } from '../logs/render';
  import { shareService as share } from '../share/share.service';
  import type { ExplorerSession, Operation } from './explorer-session';
  import { breadcrumbs, formatSize, previewLanguage, prettyText, s3Uri, type SortKey } from './format';
  import TextPreview from './TextPreview.svelte';

  /** Explorateur d'un bucket S3 : dossiers, fichiers, aperçu et opérations. */
  let { session }: { session: ExplorerSession } = $props();
  const s = untrack(() => session);
  const ROW_HEIGHT = 28;

  const crumbs = $derived(breadcrumbs(s.prefix()));
  const entries = $derived(s.entries());
  const selection = $derived(s.selectedEntries());
  /** Entrée en cours de renommage (clé). */
  let renaming = $state<string | null>(null);
  let dragging = $state(false);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  const previewText = $derived.by(() => {
    const preview = s.preview();
    const entry = s.previewed();
    if (!preview?.text || !entry) return undefined;
    const language = previewLanguage(entry.key, preview.text);
    return { language, text: prettyText(language, preview.text, preview.truncated) };
  });
  const allSelected = $derived(entries.length > 0 && s.selected().size === entries.length);

  // Premier affichage de l'onglet : chargement de la racine du bucket.
  s.onFinished = (op) => finished(op);
  if (!s.entries().length && !s.loading() && !s.error()) void s.load();

  // Liste virtuelle (lignes de hauteur fixe).
  let viewport = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(600);
  const layout = $derived(new RowLayout(entries.length, ROW_HEIGHT));
  const range = $derived(visibleRange(layout, scrollTop, height));
  $effect(() => {
    const el = viewport;
    if (!el) return;
    const observer = new ResizeObserver(() => (height = el.clientHeight));
    observer.observe(el);
    return () => observer.disconnect();
  });

  function activate(entry: S3Entry) {
    if (entry.kind === 'folder') s.open(entry.key);
    else
      void s.showPreview(entry).then(() => {
        const p = s.preview();
        if (p) analytics.event({ category: 'S3', action: 'Aperçu', name: p.gunzipped ? `${p.kind} (gzip)` : p.kind });
      });
  }

  function onSearch(text: string) {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => s.setSearch(text), 300);
  }

  function sortMark(key: SortKey): string {
    const sort = s.sort();
    return sort.key === key ? (sort.desc ? ' ▾' : ' ▴') : '';
  }

  const date = (entry: S3Entry) => (entry.modified ? new Date(entry.modified).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : '');

  function onKey(event: KeyboardEvent) {
    if (event.key === 'Delete' && selection.length) void remove(selection);
    else if (event.key === 'F2' && selection.length === 1) startRename(selection[0]);
    else if (event.key === 'Backspace' && s.prefix()) up();
    else return;
    event.preventDefault();
  }

  function up() {
    s.open(crumbs.length > 1 ? crumbs[crumbs.length - 2].prefix : '');
  }

  function download(list: S3Entry[]) {
    const label = list.length === 1 ? `Téléchargement de ${list[0].name}` : `Téléchargement (${list.length})`;
    void s.run(label, (api, ref) => api.download(ref, list));
  }

  function upload(source: 'files' | 'folder' | string[]) {
    const where = s.prefix() || 'la racine';
    void s.run(`Envoi vers ${where}`, (api, ref) => api.upload(ref, s.prefix(), source));
  }

  function commitRename(entry: S3Entry, name: string) {
    renaming = null;
    const newName = name.trim();
    if (!newName || newName === entry.name) return;
    void s.run(`Renommage de ${entry.name} en ${newName}`, (api, ref) => api.rename(ref, entry, newName));
  }

  async function remove(list: S3Entry[]) {
    let objects = list.filter((e) => e.kind === 'file').length;
    for (const folder of list.filter((e) => e.kind === 'folder')) {
      const count = await s.count(folder.key);
      if (!count.ok) return notifications.error(count.error.message);
      objects += count.value;
    }
    const names = list.length === 1 ? `« ${list[0].name} »` : `${list.length} éléments`;
    if (!confirm(`Supprimer définitivement ${names} (${objects} objet(s)) du bucket ?`)) return;
    void s.run(`Suppression de ${names}`, (api, ref) => api.delete(ref, list));
  }

  const copyPath = (entry?: S3Entry) => copy(s3Uri(s.target.bucket, entry?.key ?? s.prefix()));

  function onDragOver(event: DragEvent) {
    if (!event.dataTransfer?.types.includes('Files')) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
    dragging = true;
  }

  function onDrop(event: DragEvent) {
    event.preventDefault();
    dragging = false;
    const api = window.ocekour?.s3;
    const files = [...(event.dataTransfer?.files ?? [])];
    // Le chemin local arrive par l'événement de dépôt de la fenêtre, juste après.
    setTimeout(() => {
      const paths = api ? files.map((f) => api.pathForFile(f)).filter(Boolean) : [];
      if (paths.length) upload(paths);
    }, 50);
  }

  function startRename(entry: S3Entry) {
    renaming = entry.key;
    // Champ affiché au prochain rendu : focus et sélection du nom (sans l'extension).
    setTimeout(() => {
      const field = document.querySelector<HTMLInputElement>('input[aria-label="Nouveau nom"]');
      field?.focus();
      const dot = entry.kind === 'file' ? entry.name.lastIndexOf('.') : -1;
      field?.setSelectionRange(0, dot > 0 ? dot : entry.name.length);
    });
  }

  function finished(op: Operation) {
    const { progress } = op;
    if (progress.error) notifications.error(`${op.label} : ${progress.error.message}`);
    else if (progress.cancelled) notifications.info(`${op.label} : annulé`);
    else notifications.info(`${op.label} : terminé`);
    // Le contenu du dossier a pu changer (sauf simple téléchargement).
    if (!op.label.startsWith('Téléchargement')) void s.load();
  }
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-slate-200 px-3 py-2 text-sm dark:border-slate-800">
    <nav class="flex min-w-0 flex-wrap items-center gap-1" aria-label="Chemin">
      <span class="text-slate-500">{s.target.connectionName} /</span>
      <button type="button" class="crumb font-semibold" onclick={() => s.open('')}>{s.target.bucket}</button>
      {#each crumbs as crumb (crumb.prefix)}
        <span class="text-slate-400">/</span>
        <button type="button" class="crumb" onclick={() => s.open(crumb.prefix)}>{crumb.name}</button>
      {/each}
    </nav>
    <button type="button" class="btn px-2 py-0.5 text-xs" title="Copier le chemin s3:// du dossier" onclick={() => copyPath()}>Copier le chemin</button>
    <span class="flex-1"></span>
    <button type="button" class="btn" title="Copier un lien vers ce dossier (sans secret), à coller dans Ocekour" onclick={() => share.copyLink(s)}>🔗 Lien</button>
    <button type="button" class="btn" title="Rafraîchir" aria-label="Rafraîchir" onclick={() => s.load()}>⟳</button>
  </div>

  <div class="flex flex-wrap items-center gap-x-2 gap-y-1.5 border-b border-slate-200 px-3 py-1.5 text-sm dark:border-slate-800">
    <input type="search" class="input max-w-80 min-w-48 flex-1" placeholder="Rechercher (début du nom)…" aria-label="Rechercher dans ce dossier" value={s.search()} oninput={(e) => onSearch(e.currentTarget.value)} />
    <button type="button" class="btn" onclick={() => upload('files')}>↑ Envoyer des fichiers…</button>
    <button type="button" class="btn" onclick={() => upload('folder')}>↑ Envoyer un dossier…</button>
    {#if selection.length}
      <span class="ml-2 text-xs text-slate-500">{selection.length} sélectionné(s) :</span>
      <button type="button" class="btn" onclick={() => download(selection)}>↓ Télécharger</button>
      {#if selection.length === 1}<button type="button" class="btn" title="F2" onclick={() => startRename(selection[0])}>Renommer</button>{/if}
      <button type="button" class="btn text-red-600 dark:text-red-400" title="Suppr" onclick={() => remove(selection)}>Supprimer</button>
    {/if}
  </div>

  <div class="flex min-h-0 flex-1">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="relative flex min-w-0 flex-1 flex-col" ondragover={onDragOver} ondragleave={() => (dragging = false)} ondrop={onDrop}>
      <div class="flex h-7 shrink-0 items-center gap-3 border-b border-slate-200 bg-slate-50 px-3 text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-900">
        <input type="checkbox" aria-label="Tout sélectionner" checked={allSelected} onchange={(e) => s.selectAll(e.currentTarget.checked)} />
        <button type="button" class="flex-1 text-left" onclick={() => s.setSort('name')}>Nom{sortMark('name')}</button>
        <button type="button" class="w-24 text-right" onclick={() => s.setSort('size')}>Taille{sortMark('size')}</button>
        <button type="button" class="w-36 text-left" onclick={() => s.setSort('modified')}>Modifié le{sortMark('modified')}</button>
      </div>

      {#if s.error()}
        <div role="alert" class="border-b border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-300">
          {s.error()?.message}
          <button type="button" class="ml-2 underline" onclick={() => s.load()}>Réessayer</button>
        </div>
      {/if}

      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div bind:this={viewport} tabindex="0" role="list" aria-label="Contenu du dossier" class="min-h-0 flex-1 overflow-y-auto text-sm focus-visible:outline-none" onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)} onkeydown={onKey}>
        {#if s.prefix()}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="flex cursor-pointer items-center gap-3 px-3 text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800" style:height="{ROW_HEIGHT}px" onclick={up}>
            <span class="w-3.5"></span>
            <span>📁 ..</span>
          </div>
        {/if}
        <div class="relative" style:height="{layout.total}px">
          <div class="absolute inset-x-0" style:top="{layout.offsetOf(range.start)}px">
            {#each entries.slice(range.start, range.end) as entry (entry.key)}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div
                class="group flex cursor-pointer items-center gap-3 px-3 hover:bg-slate-100 dark:hover:bg-slate-800"
                class:bg-sky-100!={s.previewed()?.key === entry.key}
                class:dark:bg-sky-900!={s.previewed()?.key === entry.key}
                style:height="{ROW_HEIGHT}px"
                data-key={entry.key}
                onclick={() => activate(entry)}
              >
                <input type="checkbox" aria-label={'Sélectionner ' + entry.name} checked={s.selected().has(entry.key)} onclick={(e) => e.stopPropagation()} onchange={() => s.toggleSelected(entry.key)} />
                {#if renaming === entry.key}
                  <input
                    class="input min-w-0 flex-1 py-0"
                    aria-label="Nouveau nom"
                    value={entry.name}
                    onclick={(e) => e.stopPropagation()}
                    onkeydown={(e) => {
                      e.stopPropagation();
                      if (e.key === 'Enter') commitRename(entry, e.currentTarget.value);
                      else if (e.key === 'Escape') renaming = null;
                    }}
                    onblur={() => renaming === entry.key && (renaming = null)}
                  />
                {:else}
                  <span class="min-w-0 flex-1 truncate" title={entry.key}>{entry.kind === 'folder' ? '📁' : '📄'} {entry.name}</span>
                {/if}
                <span class="w-24 shrink-0 text-right text-xs text-slate-500 tabular-nums">{formatSize(entry.size)}</span>
                <span class="w-36 shrink-0 text-xs text-slate-500 tabular-nums">{date(entry)}</span>
              </div>
            {/each}
          </div>
        </div>
      </div>

      <div class="flex h-8 shrink-0 items-center gap-3 border-t border-slate-200 px-3 text-xs text-slate-500 dark:border-slate-800">
        {#if s.loading()}
          <span>Chargement…</span>
        {:else}
          <span>{entries.length.toLocaleString('fr-FR')} élément(s){s.next() ? ' (liste partielle)' : ''}</span>
          {#if !entries.length && !s.error()}<span>· {s.search() ? 'aucun résultat' : 'dossier vide'}</span>{/if}
        {/if}
        {#if s.next() && !s.loading()}<button type="button" class="btn py-0 text-xs" onclick={() => s.loadMore()}>Charger les 1 000 suivants</button>{/if}
        <span class="ml-auto">Glissez des fichiers ici pour les envoyer</span>
      </div>

      {#if dragging}
        <div class="pointer-events-none absolute inset-0 flex items-center justify-center border-2 border-dashed border-sky-500 bg-sky-500/10 text-sm font-medium text-sky-700 dark:text-sky-300">
          Déposer pour envoyer dans {s.prefix() || 'la racine du bucket'}
        </div>
      {/if}
    </div>

    {#if s.previewed()}
      {@const entry = s.previewed()!}
      {@const p = s.preview()}
      <section class="flex w-[45%] min-w-72 shrink-0 flex-col border-l border-slate-200 dark:border-slate-800" aria-label="Aperçu">
        <div class="flex flex-wrap items-center gap-2 border-b border-slate-200 px-3 py-1.5 text-sm dark:border-slate-800">
          <strong class="min-w-0 flex-1 truncate" title={entry.key}>{entry.name}</strong>
          <button type="button" class="btn py-0.5" onclick={() => download([entry])}>↓ Télécharger</button>
          <button type="button" class="btn py-0.5" onclick={() => copyPath(entry)}>Copier le chemin</button>
          <button type="button" class="btn px-2 py-0.5" aria-label="Fermer l'aperçu" onclick={() => s.closePreview()}>✕</button>
        </div>
        <div class="flex flex-wrap gap-x-3 px-3 py-1 text-xs text-slate-500">
          <span>{formatSize(entry.size)}</span>
          <span>{date(entry)}</span>
          {#if p?.contentType}<span>{p.contentType}</span>{/if}
          {#if p?.gunzipped}<span>décompressé (gzip)</span>{/if}
        </div>
        {#if p?.truncated}
          <p class="mx-3 mb-1 rounded bg-amber-50 px-2 py-1 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-200">Aperçu limité au début du fichier (5 Mo). Téléchargez-le pour tout voir.</p>
        {/if}
        <div class="flex min-h-0 flex-1 flex-col">
          {#if s.previewLoading()}
            <p class="m-auto text-sm text-slate-500">Chargement de l'aperçu…</p>
          {:else if s.previewError()}
            <p class="m-3 text-sm text-red-600">{s.previewError()?.message}</p>
          {:else if p}
            {#if p.kind === 'image'}
              <div class="min-h-0 flex-1 overflow-auto p-3"><img src={p.dataUrl} alt={entry.name} class="max-w-full" /></div>
            {:else if p.kind === 'text'}
              {#if previewText}
                <TextPreview text={previewText.text} language={previewText.language} />
              {:else}
                <p class="m-3 text-sm text-slate-500">Fichier vide.</p>
              {/if}
            {:else}
              <p class="m-auto max-w-xs text-center text-sm text-slate-500">Fichier binaire : pas d'aperçu possible. Téléchargez-le pour l'ouvrir.</p>
            {/if}
          {/if}
        </div>
      </section>
    {/if}
  </div>

  {#if s.operations().length}
    <ul class="max-h-40 shrink-0 overflow-y-auto border-t border-slate-200 bg-slate-50 px-3 py-1.5 text-xs dark:border-slate-800 dark:bg-slate-900" aria-label="Opérations en cours">
      {#each s.operations() as op (op.opId)}
        {@const p = op.progress}
        <li class="flex items-center gap-3 py-0.5">
          <span class="w-72 truncate" title={op.label}>{op.label}</span>
          <progress class="h-2 w-40" value={p.done} max={p.total || 1}></progress>
          <span class="min-w-0 flex-1 truncate text-slate-500" title={p.current ?? ''}>
            {#if p.error}<span class="text-red-600">{p.error.message}</span>{:else if p.cancelled}annulé{:else if p.finished}terminé{:else}{p.done}{p.total !== undefined ? ' / ' + p.total : ''} · {formatSize(p.bytes)} {p.current}{/if}
          </span>
          {#if p.finished}
            <button type="button" class="btn px-1.5 py-0" onclick={() => s.dismiss(op.opId)}>✕</button>
          {:else}
            <button type="button" class="btn py-0" onclick={() => s.cancel(op.opId)}>Annuler</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>
