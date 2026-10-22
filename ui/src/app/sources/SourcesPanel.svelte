<script lang="ts">
  import type { Connection, SourcePath } from '../../../shared/api';
  import ConnectionDialog from '../connections/ConnectionDialog.svelte';
  import { connectionDialog as dialog } from '../connections/connection-dialog.service';
  import { configService } from '../core/config.svelte';
  import { notifications } from '../core/notification.svelte';
  import { tabsService as tabs } from '../logs/tabs.service';
  import { type TabTarget, targetKey } from '../logs/target';
  import { shareService as share } from '../share/share.service';
  import { nodeKey } from './source-tree.service';
  import { sourceTree as tree } from './source-tree.svelte';
  import LocalFilesSection from '../files/LocalFilesSection.svelte';
  import { age, size } from '../files/format';
  import { favoriteRows as buildFavorites, type Row, treeRows } from './tree-rows';

  /** Barre latérale : connexions Rancher (clusters › namespaces › pods › conteneurs), Kafka (topics), S3 (buckets). */
  const available = !!window.ocekour;
  let filter = $state('');

  const connections = $derived(configService.config?.connections ?? []);
  const treeInput = $derived({
    connections,
    nodes: tree.nodes,
    isExpanded: (t: SourcePath) => tree.isExpanded(t),
    isFavoriteOpen: (f: string) => tree.isFavoriteOpen(f),
    favorites: new Set(configService.config?.favorites ?? []),
    filter,
  });
  const rows = $derived(treeRows(treeInput));
  const favoriteRows = $derived(buildFavorites(treeInput));
  const activeKey = $derived.by(() => {
    const active = tabs.activeTarget();
    return active ? targetKey(active) : undefined;
  });

  function openDialog(connection: Connection | null, focusToken = false) {
    dialog.open(connection, { focusToken });
  }

  function connectionOf(row: Row): Connection | undefined {
    return connections.find((c) => c.id === row.target.connectionId);
  }

  /** Clic : ouvre dans l'onglet courant ; Ctrl+clic ou clic milieu : dans un nouvel onglet. */
  function activate(row: Row, event?: MouseEvent | KeyboardEvent) {
    if (row.kind === 'favorite') {
      if (row.expandable) tree.toggleFavoriteOpen(row.favoriteKey!, row.target);
      return;
    }
    if (row.expandable) {
      tree.toggle(row.target, row.kind === 'group');
      return;
    }
    openRow(row, event);
  }

  function openRow(row: Row, event?: MouseEvent | KeyboardEvent) {
    const target = targetOf(row);
    if (!target) return;
    const newTab = !!event && (event.ctrlKey || event.metaKey || ('button' in event && event.button === 1));
    // Un assemblage est actif : un clic simple y ajoute la source (Ctrl/clic milieu : onglet à part).
    if (!newTab && target.kind !== 'bucket' && target.kind !== 'assembly') {
      if (tabs.addToAssembly(target)) return;
    }
    tabs.open(target, newTab);
  }

  /** Cible d'onglet d'une feuille de l'arbre (conteneur, topic, bucket). */
  function targetOf(row: Row): TabTarget | undefined {
    const connection = connectionOf(row);
    if (!connection) return undefined;
    const { connectionId, path } = row.target;
    if (row.kind === 'topic' || row.kind === 'bucket') {
      const base = { connectionId, connectionName: connection.name };
      return row.kind === 'topic'
        ? { kind: 'topic', ...base, topic: path[path.length - 1] }
        : { kind: 'bucket', ...base, bucket: path[path.length - 1] };
    }
    if (connection.type === 'sonde' && (row.kind === 'source' || row.kind === 'pattern' || row.kind === 'file')) {
      // Source (ou motif suivi sur son fichier le plus récent), ou fichier d'un motif.
      const source = path[0];
      const node = tree.nodes.get(nodeKey({ connectionId, path: [] }))?.children?.find((n) => n.id === source);
      return {
        kind: 'file',
        connectionId,
        connectionName: connection.name,
        path: row.kind === 'file' ? [source, path[1]] : [source],
        name: node?.state || source,
        pattern: row.kind === 'pattern',
      };
    }
    if (row.kind !== 'container' || connection.type !== 'rancher') return undefined;
    const [clusterId, namespace, pod, container] = path;
    const podNode = tree.nodes.get(nodeKey({ connectionId, path: [clusterId, namespace] }))?.children?.find((n) => n.id === pod);
    const cluster = connection.clusters.find((c) => c.id === clusterId);
    return {
      kind: 'container',
      connectionId,
      clusterId,
      namespace,
      pod,
      container,
      node: row.node!,
      clusterName: cluster?.name || clusterId,
      workload: podNode?.workload || pod,
    };
  }

  function isSelected(row: Row): boolean {
    const target = (!row.expandable || row.kind === 'pattern') && row.kind !== 'favorite' ? targetOf(row) : undefined;
    return !!activeKey && !!target && activeKey === targetKey(target);
  }

  async function toggleFavorite(row: Row) {
    if (!row.favoriteKey) return;
    try {
      await window.ocekour?.config.toggleFavorite(row.favoriteKey);
    } catch (e) {
      notifications.error(e);
    }
  }

  const healthClass = (h: string) => (h === 'ok' ? 'bg-emerald-500' : h === 'warn' ? 'bg-amber-500' : h === 'error' ? 'bg-red-500' : 'bg-slate-400');
  const typeClass = (t: string) =>
    t === 'kafka'
      ? 'bg-violet-100 text-violet-700 dark:bg-violet-900/60 dark:text-violet-300'
      : t === 's3'
        ? 'bg-amber-100 text-amber-700 dark:bg-amber-900/60 dark:text-amber-300'
        : t === 'sonde'
          ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/60 dark:text-emerald-300'
          : 'bg-sky-100 text-sky-700 dark:bg-sky-900/60 dark:text-sky-300';
</script>

{#snippet rowTpl(row: Row)}
  {@const selected = isSelected(row)}
  <li role="treeitem" aria-expanded={row.expandable ? row.expanded : undefined} aria-selected={selected} aria-level={row.depth + 1}>
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
    <div
      tabindex="0"
      class="group flex h-7 cursor-pointer items-center gap-1.5 pr-2 hover:bg-slate-200/70 focus-visible:outline-2 focus-visible:outline-sky-500 dark:hover:bg-slate-800 {selected
        ? 'bg-sky-100 dark:bg-sky-900/50'
        : ''}"
      style:padding-left="{6 + row.depth * 14}px"
      onclick={(e) => activate(row, e)}
      onauxclick={(e) => e.button === 1 && activate(row, e)}
      onmousedown={(e) => e.button === 1 && e.preventDefault()}
      onkeydown={(e) => e.key === 'Enter' && activate(row, e)}
    >
      <span class="w-3 shrink-0 text-center text-xs text-slate-400">
        {#if row.loading}<span class="inline-block animate-spin">◌</span>{:else if row.expandable}{row.expanded ? '▾' : '▸'}{/if}
      </span>
      {#if row.node?.health}
        <span class="size-2 shrink-0 rounded-full {healthClass(row.node.health)}" title={row.node?.state}></span>
      {/if}
      {#if row.connectionType}
        <span class="shrink-0 rounded px-1 text-[10px] font-semibold uppercase {typeClass(row.connectionType)}"
          >{row.connectionType === 'rancher' ? 'k8s' : row.connectionType}</span
        >
      {/if}
      <span
        class="min-w-0 flex-1 truncate"
        class:font-semibold={row.kind === 'connection' || row.kind === 'favorite'}
        class:text-slate-400={row.missing}
        class:text-slate-500={row.kind === 'container' && row.node?.init}
        title={row.label + (row.node?.state ? ' — ' + row.node?.state : '')}
      >
        {row.kind === 'source' || row.kind === 'pattern' ? row.node?.state || row.label : row.label}
        {#if row.detail}<span class="ml-1 text-xs text-slate-400">{row.detail}</span>{/if}
        {#if row.missing}<span class="ml-1 text-xs text-slate-400 italic">(introuvable)</span>{/if}
      </span>
      {#if row.kind === 'pod'}
        {#if row.node?.restarts}<span class="shrink-0 text-xs text-amber-600" title="Redémarrages">↻{row.node?.restarts}</span>{/if}
        <span class="shrink-0 text-xs text-slate-400 group-hover:hidden">{age(row.node?.createdAt)}</span>
      {/if}
      {#if row.kind === 'source' || row.kind === 'pattern' || row.kind === 'file'}
        <span class="shrink-0 text-xs text-slate-400 group-hover:hidden">{size(row.node?.size)} {age(row.node?.modified)}</span>
      {/if}
      {#if row.kind === 'pattern'}
        <button
          type="button"
          class="hidden shrink-0 rounded px-1 text-xs text-sky-600 group-hover:inline hover:bg-slate-300 dark:hover:bg-slate-700"
          title="Suivre le fichier le plus récent (bascule à chaque nouveau fichier)"
          onclick={(e) => {
            e.stopPropagation();
            openRow(row, e);
          }}>▶ suivre</button
        >
      {/if}
      {#if row.kind === 'container'}
        {#if row.node?.init}<span class="shrink-0 rounded bg-slate-200 px-1 text-[10px] text-slate-600 dark:bg-slate-700 dark:text-slate-300">init</span>{/if}
        {#if row.node?.state !== 'Running'}<span class="shrink-0 truncate text-xs text-slate-500">{row.node?.state}</span>{/if}
      {/if}
      {#if row.favoriteKey}
        <button
          type="button"
          class="shrink-0 text-xs {row.favorite ? 'text-amber-500' : 'hidden text-slate-400 group-hover:inline hover:text-amber-500'}"
          title={row.favorite ? 'Retirer des favoris' : row.kind === 'pod' ? 'Épingler le workload ' + row.node?.workload : 'Épingler le namespace'}
          onclick={(e) => {
            e.stopPropagation();
            void toggleFavorite(row);
          }}>{row.favorite ? '★' : '☆'}</button
        >
      {/if}
      {#if row.kind === 'connection'}
        <button
          type="button"
          class="hidden shrink-0 rounded px-1.5 text-xs text-slate-500 group-hover:inline hover:bg-slate-300 dark:hover:bg-slate-700"
          onclick={(e) => {
            e.stopPropagation();
            openDialog(connectionOf(row) ?? null);
          }}>Modifier</button
        >
      {:else if row.expandable && row.expanded}
        <button
          type="button"
          class="hidden shrink-0 text-xs text-slate-400 group-hover:inline hover:text-slate-700 dark:hover:text-slate-200"
          title="Rafraîchir"
          onclick={(e) => {
            e.stopPropagation();
            void tree.load(row.target);
          }}>⟳</button
        >
      {/if}
    </div>
    {#if row.error}
      <div role="alert" class="flex flex-wrap items-center gap-x-2 py-1 pr-2 text-xs text-red-600 dark:text-red-400" style:padding-left="{26 + row.depth * 14}px">
        <span>{row.error.message}</span>
        {#if row.error.code === 'unauthorized'}
          <button type="button" class="underline" onclick={() => openDialog(connectionOf(row) ?? null, true)}>Mettre à jour les identifiants</button>
        {:else}
          <button type="button" class="underline" onclick={() => tree.load(row.target)}>Réessayer</button>
        {/if}
      </div>
    {/if}
  </li>
{/snippet}

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex items-center gap-1.5 p-2">
    <input type="search" class="input min-w-0 flex-1" placeholder="Filtrer (pods, topics, fichiers…)" aria-label="Filtrer l'arborescence" bind:value={filter} />
    {#if available}
      <button type="button" class="btn px-2" title="Rafraîchir" aria-label="Rafraîchir" onclick={() => tree.refreshAll()}>⟳</button>
      <button type="button" class="btn px-2" title="Ouvrir le lien Ocekour copié (ou Ctrl+V n'importe où)" aria-label="Ouvrir un lien" onclick={() => share.openFromClipboard()}>🔗</button>
      <button type="button" class="btn px-2" title="Nouvelle connexion" aria-label="Nouvelle connexion" onclick={() => openDialog(null)}>+</button>
    {/if}
  </div>

  {#if !connections.length}
    <p class="px-3 text-sm text-slate-500">
      Aucune connexion.
      {#if available}<button type="button" class="text-sky-600 hover:underline" onclick={() => openDialog(null)}>Ajouter une connexion</button>{/if}
    </p>
  {/if}

  <div class="min-h-0 flex-1 overflow-y-auto pb-2 text-sm">
    {#if available}<LocalFilesSection {filter} />{/if}
    {#if favoriteRows.length}
      <h2 class="px-3 pt-1 pb-0.5 text-xs font-semibold tracking-wide text-amber-600 uppercase">★ Favoris</h2>
      <ul role="tree" aria-label="Favoris" class="mb-2 border-b border-slate-200 pb-2 dark:border-slate-800">
        {#each favoriteRows as row (row.key)}{@render rowTpl(row)}{/each}
      </ul>
    {/if}
    <ul role="tree" aria-label="Sources">
      {#each rows as row (row.key)}{@render rowTpl(row)}{/each}
    </ul>
  </div>
</div>

{#if dialog.request()}
  {@const edit = dialog.request()!}
  <ConnectionDialog
    connection={edit.connection}
    focusToken={edit.focusToken}
    prefill={edit.prefill}
    onclose={() => {
      dialog.close();
      tree.refreshAll();
    }}
  />
{/if}
