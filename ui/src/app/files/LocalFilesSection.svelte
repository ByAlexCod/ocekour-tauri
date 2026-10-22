<script lang="ts">
  import { LOCAL_CONNECTION } from '../../../shared/api';
  import { tabsService as tabs } from '../logs/tabs.service';
  import { targetKey } from '../logs/target';
  import { nodeKey } from '../sources/source-tree.service';
  import { sourceTree as tree } from '../sources/source-tree.svelte';
  import { age, size } from './format';
  import { isPattern, localFiles, localTarget } from './local-files.svelte';

  /** Section « Fichiers locaux » : favoris et récents ; un motif se déplie sur ses fichiers. */
  let { filter = '' }: { filter?: string } = $props();

  const activeKey = $derived.by(() => {
    const active = tabs.activeTarget();
    return active ? targetKey(active) : undefined;
  });
  const shown = $derived(localFiles.list.filter((f) => !filter.trim() || f.path.toLowerCase().includes(filter.trim().toLowerCase())));

  const newTab = (e: MouseEvent | KeyboardEvent) => e.ctrlKey || e.metaKey || ('button' in e && e.button === 1);
  const patternTarget = (path: string) => ({ connectionId: LOCAL_CONNECTION, path: [path] });
  const selected = (path: string, file?: string) => activeKey === targetKey(localTarget(path, { file }));

  // Saisie d'un motif (`C:\app\logs\app-*.log`) : la boîte « Ouvrir » ne choisit que des fichiers.
  let patternOpen = $state(false);
  let patternText = $state('');
  const patternValid = $derived(isPattern(patternText.trim()) && /^([a-zA-Z]:[\\/]|[\\/])/.test(patternText.trim()));

  function openPattern(event: SubmitEvent) {
    event.preventDefault();
    if (!patternValid) return;
    localFiles.open(patternText.trim(), tabs.tabs().length > 0);
    patternText = '';
    patternOpen = false;
  }
</script>

<section aria-label="Fichiers locaux" class="mb-2 border-b border-slate-200 pb-2 dark:border-slate-800">
  <div class="flex items-center px-3 pt-1 pb-0.5">
    <h2 class="flex-1 text-xs font-semibold tracking-wide text-slate-500 uppercase">📄 Fichiers locaux</h2>
    <button type="button" class="mr-2 text-xs text-sky-600 hover:underline" title="Suivre une famille de fichiers (app-*.log) : le plus récent, avec bascule à la rotation" onclick={() => (patternOpen = !patternOpen)}>Motif…</button>
    <button type="button" class="text-xs text-sky-600 hover:underline" title="Ouvrir un fichier de logs (Ctrl+O)" onclick={() => localFiles.pick()}>Ouvrir…</button>
  </div>
  {#if patternOpen}
    <form class="flex items-center gap-1 px-2 pb-1" onsubmit={openPattern}>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input min-w-0 flex-1 py-0.5 font-mono text-xs"
        placeholder="C:\app\logs\app-*.log"
        aria-label="Motif de fichiers"
        title="Chemin complet ; jokers * ? [0-9] dans le nom du fichier seulement"
        autofocus
        bind:value={patternText}
        onkeydown={(e) => e.key === 'Escape' && (patternOpen = false)}
      />
      <button type="submit" class="btn py-0.5 text-xs" disabled={!patternValid}>Suivre</button>
    </form>
  {/if}
  {#if !localFiles.list.length}
    <p class="px-3 text-xs text-slate-400">Ctrl+O, ou glissez des fichiers de logs sur la fenêtre.</p>
  {/if}
  <ul role="tree" aria-label="Fichiers locaux">
    {#each shown as f (f.path)}
      {@const expanded = f.pattern && tree.isExpanded(patternTarget(f.path))}
      {@const state = f.pattern ? tree.nodes.get(nodeKey(patternTarget(f.path))) : undefined}
      <li role="treeitem" aria-selected={selected(f.path)} aria-expanded={f.pattern ? expanded : undefined}>
        <!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
        <div
          tabindex="0"
          class="group flex h-7 cursor-pointer items-center gap-1.5 pr-2 pl-1.5 hover:bg-slate-200/70 dark:hover:bg-slate-800 {selected(f.path) ? 'bg-sky-100 dark:bg-sky-900/50' : ''}"
          title={f.path}
          onclick={(e) => localFiles.open(f.path, newTab(e))}
          onauxclick={(e) => e.button === 1 && localFiles.open(f.path, true)}
          onmousedown={(e) => e.button === 1 && e.preventDefault()}
          onkeydown={(e) => e.key === 'Enter' && localFiles.open(f.path, newTab(e))}
        >
          <button
            type="button"
            class="w-3 shrink-0 text-center text-xs text-slate-400"
            aria-label={expanded ? 'Replier' : 'Fichiers du motif'}
            disabled={!f.pattern}
            onclick={(e) => {
              e.stopPropagation();
              tree.toggle(patternTarget(f.path));
            }}
          >
            {#if state?.loading}<span class="inline-block animate-spin">◌</span>{:else if f.pattern}{expanded ? '▾' : '▸'}{/if}
          </button>
          <span class="size-2 shrink-0 rounded-full {f.exists ? 'bg-emerald-500' : 'bg-slate-400'}"></span>
          <span class="min-w-0 flex-1 truncate" class:text-slate-400={!f.exists}>
            {f.name}
            {#if !f.exists}<span class="ml-1 text-xs italic">(introuvable)</span>{/if}
          </span>
          {#if f.encoding}<span class="shrink-0 text-[10px] text-slate-400 uppercase">{f.encoding}</span>{/if}
          <span class="shrink-0 text-xs text-slate-400 group-hover:hidden">{size(f.size)} {age(f.modified)}</span>
          <button
            type="button"
            class="shrink-0 text-xs {f.favorite ? 'text-amber-500' : 'hidden text-slate-400 group-hover:inline hover:text-amber-500'}"
            title={f.favorite ? 'Retirer des favoris' : 'Garder dans la liste'}
            onclick={(e) => {
              e.stopPropagation();
              localFiles.favorite(f.path, !f.favorite);
            }}>{f.favorite ? '★' : '☆'}</button
          >
          <button
            type="button"
            class="hidden shrink-0 text-xs text-slate-400 group-hover:inline hover:text-red-600"
            title="Retirer de la liste"
            onclick={(e) => {
              e.stopPropagation();
              localFiles.forget(f.path);
            }}>✕</button
          >
        </div>
        {#if expanded}
          {#if state?.error}
            <p role="alert" class="py-1 pr-2 pl-8 text-xs text-red-600">{state.error.message}</p>
          {/if}
          <ul role="group">
            {#each state?.children ?? [] as file (file.id)}
              <li role="treeitem" aria-selected={selected(f.path, file.id)}>
                <!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
                <div
                  tabindex="0"
                  class="group flex h-7 cursor-pointer items-center gap-1.5 pr-2 pl-8 hover:bg-slate-200/70 dark:hover:bg-slate-800 {selected(f.path, file.id) ? 'bg-sky-100 dark:bg-sky-900/50' : ''}"
                  onclick={(e) => localFiles.open(f.path, newTab(e), file.id)}
                  onauxclick={(e) => e.button === 1 && localFiles.open(f.path, true, file.id)}
                  onkeydown={(e) => e.key === 'Enter' && localFiles.open(f.path, newTab(e), file.id)}
                >
                  <span class="min-w-0 flex-1 truncate">{file.id}</span>
                  <span class="shrink-0 text-xs text-slate-400">{size(file.size)} {age(file.modified)}</span>
                </div>
              </li>
            {:else}
              {#if !state?.loading}<li class="py-1 pl-8 text-xs text-slate-400">Aucun fichier ne correspond.</li>{/if}
            {/each}
          </ul>
        {/if}
      </li>
    {/each}
  </ul>
</section>
