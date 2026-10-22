<script lang="ts">
  import { type Tab, tabsService as tabs } from './tabs.service';
  import { targetPath, targetTitle } from './target';

  /** Barre d'onglets : un onglet par flux, réordonnable par glisser-déposer. */
  let dragFrom = $state<number | null>(null);

  function dot(tab: Tab): string {
    switch (tab.status().state) {
      case 'live':
        return 'bg-emerald-500';
      case 'error':
        return 'bg-red-500';
      case 'ended':
        return 'bg-slate-400';
      default:
        return 'bg-amber-500';
    }
  }

  function drop(to: number) {
    if (dragFrom !== null && dragFrom !== to) tabs.move(dragFrom, to);
    dragFrom = null;
  }
</script>

<div role="tablist" aria-label="Flux ouverts" class="flex min-w-0 overflow-x-auto border-b border-slate-200 bg-slate-50 dark:border-slate-800 dark:bg-slate-900">
  {#each tabs.tabs() as tab, index (tab)}
    {@const active = tab === tabs.active()}
    {@const t = targetTitle(tab.target)}
    <div
      draggable="true"
      role="tab"
      tabindex="0"
      aria-selected={active}
      title={`${targetPath(tab.target).join(' / ')}\nClic milieu : fermer`}
      class="group flex h-9 max-w-64 min-w-32 shrink-0 cursor-pointer items-center gap-2 border-r border-slate-200 px-3 text-sm select-none dark:border-slate-800 {active
        ? 'bg-white text-slate-900 shadow-[inset_0_2px_0_var(--color-sky-500)] dark:bg-slate-950 dark:text-slate-100'
        : 'text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800'}"
      onclick={() => tabs.active.set(tab)}
      onkeydown={(e) => e.key === 'Enter' && tabs.active.set(tab)}
      onauxclick={(e) => e.button === 1 && tabs.close(tab)}
      ondragstart={(e) => {
        dragFrom = index;
        e.dataTransfer?.setData('text/plain', String(index));
      }}
      ondragover={(e) => e.preventDefault()}
      ondrop={(e) => {
        e.preventDefault();
        drop(index);
      }}
    >
      <span class="size-2 shrink-0 rounded-full {dot(tab)}"></span>
      <span class="min-w-0 flex-1 truncate">
        <span class="font-medium">{t.title}</span>
        <span class="text-xs opacity-70"> · {t.subtitle}</span>
      </span>
      {#if tab.unseenErrors()}
        <span class="shrink-0 rounded-full bg-red-600 px-1.5 text-[10px] font-semibold text-white" title="{tab.unseenErrors()} nouvelle(s) erreur(s)"
          >{tab.unseenErrors() > 99 ? '99+' : tab.unseenErrors()}</span
        >
      {/if}
      <button
        type="button"
        class="shrink-0 rounded px-1 text-slate-400 opacity-0 group-hover:opacity-100 hover:bg-slate-200 hover:text-slate-700 dark:hover:bg-slate-700 dark:hover:text-slate-200"
        class:opacity-100={active}
        aria-label="Fermer l'onglet (Ctrl+W)"
        onclick={(e) => {
          e.stopPropagation();
          tabs.close(tab);
        }}>✕</button
      >
    </div>
  {/each}
  <button
    type="button"
    class="flex h-9 shrink-0 items-center gap-1 px-3 text-sm text-slate-500 hover:bg-slate-100 hover:text-sky-600 dark:hover:bg-slate-800"
    title="Nouvel assemblage : fusionner plusieurs pods / topics dans une vue triée"
    onclick={() => tabs.startAssembly()}><span class="text-base leading-none">⊕</span> Assemblage</button
  >
</div>
