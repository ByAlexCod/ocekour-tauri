<script lang="ts">
  import { copy } from '../../core/clipboard';
  import { type FieldRole, fieldActions } from '../field-actions';
  import { type Matcher, segments as split } from '../filter';
  import { jsonMatches } from './json-node';
  import Self from './JsonNode.svelte';

  /**
   * Nœud de l'arbre JSON du détail. Les chaînes multi-lignes (stack traces, SQL…) sont
   * affichées telles quelles, sur plusieurs lignes.
   */
  let {
    key = null,
    value,
    path = '',
    depth = 0,
    filter = '',
    highlight,
  }: {
    key?: string | number | null;
    value: unknown;
    /** Chemin pointé depuis la racine (`payload.client.id`). */
    path?: string;
    depth?: number;
    /** Filtre de recherche, en minuscules. */
    filter?: string;
    /** Recherche du flux, surlignée dans les valeurs. */
    highlight?: Matcher;
  } = $props();

  const actions = fieldActions();
  const roles: { role: FieldRole; label: string }[] = [
    { role: 'time', label: 'heure' },
    { role: 'level', label: 'niveau' },
    { role: 'message', label: 'message' },
  ];
  let menuOpen = $state(false);

  const kind = $derived(value === null ? 'null' : Array.isArray(value) ? 'array' : (typeof value as 'object' | 'string' | 'number' | 'boolean'));
  const isCollection = $derived(kind === 'object' || kind === 'array');
  /** Racine de l'arbre : pas de ligne d'en-tête, enfants affichés directement. */
  const isRoot = $derived(key === null && depth === 0);
  /** Déplié par défaut sur les premiers niveaux, et toujours pendant une recherche. */
  let toggled = $state<boolean | undefined>(undefined);
  const defaultExpanded = $derived(isRoot || depth < 2 || !!filter);
  $effect(() => {
    // Recherche changée : retour au dépliage par défaut.
    void defaultExpanded;
    toggled = undefined;
  });
  const expanded = $derived(toggled ?? defaultExpanded);

  const entries = $derived.by(() => {
    if (!isCollection) return [];
    const v = value as Record<string, unknown> | unknown[];
    const all = Array.isArray(v) ? v.map((child, i) => ({ key: i as string | number, value: child })) : Object.entries(v).map(([k, val]) => ({ key: k as string | number, value: val }));
    return filter ? all.filter((e) => jsonMatches(e.key, e.value, filter)) : all;
  });
  const summary = $derived.by(() => {
    if (!isCollection) return '';
    const v = value as object;
    const n = Array.isArray(v) ? v.length : Object.keys(v).length;
    return Array.isArray(v) ? `[ ${n} élément(s) ]` : `{ ${n} champ(s) }`;
  });
  const multiline = $derived(kind === 'string' && /\r?\n/.test(value as string));
  /** Les réglages de profil ne portent que sur des champs d'objet (pas des index de tableau). */
  const configurable = $derived(!!actions && !!path && !path.includes('['));

  function childPath(k: string | number): string {
    if (typeof k === 'number') return `${path}[${k}]`;
    return path ? `${path}.${k}` : k;
  }

  function closeMenuOnBlur(event: FocusEvent) {
    const menu = event.currentTarget as HTMLElement;
    if (!menu.contains(event.relatedTarget as Node | null)) menuOpen = false;
  }

  function run(action: () => unknown) {
    menuOpen = false;
    action();
  }

  const copyValue = () => copy(typeof value === 'string' ? value : JSON.stringify(value, null, 2), 'Valeur copiée');
  const copyPath = () => copy(path, 'Chemin copié');
</script>

<div>
  {#if !isRoot}
    <div class="group/row flex items-start gap-1 rounded px-1 hover:bg-slate-100 dark:hover:bg-slate-800/70">
      {#if isCollection}
        <button type="button" class="w-3 shrink-0 text-slate-400" aria-expanded={expanded} onclick={() => (toggled = !expanded)}>{expanded ? '▾' : '▸'}</button>
      {:else}
        <span class="w-3 shrink-0"></span>
      {/if}

      <div class="min-w-0 flex-1">
        {#if key !== null}
          <span class="text-violet-700 dark:text-violet-300">{key}</span>
          {#if actions?.isExpanded(path)}<span class="text-violet-500" title="Mis en avant pour ce workload">★</span>{/if}
          {#if actions?.isColumn(path)}<span class="text-sky-500" title="Affiché en colonne">▥</span>{/if}
          <span class="text-slate-400">: </span>
        {/if}
        {#if kind === 'string'}
          {#if !multiline}
            <span class="json-string break-all text-emerald-700 dark:text-emerald-300"
              >{#each split(value as string, highlight) as seg, i (i)}<span class:hl={seg.match}>{seg.text}</span>{/each}</span
            >
          {/if}
        {:else if kind === 'number'}
          <span class="text-sky-700 dark:text-sky-300">{value}</span>
        {:else if kind === 'boolean'}
          <span class="text-amber-700 dark:text-amber-300">{value}</span>
        {:else if kind === 'null'}
          <span class="text-slate-400">null</span>
        {:else if !expanded}
          <button type="button" class="text-slate-400 hover:underline" onclick={() => (toggled = true)}>{summary}</button>
        {/if}
      </div>

      <span class="relative shrink-0 gap-1 group-hover/row:flex {menuOpen ? 'flex' : 'hidden'}" onfocusout={closeMenuOnBlur}>
        <button type="button" class="detail-action" title="Copier la valeur" onclick={copyValue}>⧉</button>
        {#if configurable && actions}
          <button type="button" class="detail-action" title="Réglages de ce champ pour le workload" aria-haspopup="menu" aria-expanded={menuOpen} onclick={() => (menuOpen = !menuOpen)}>⋯</button>
          {#if menuOpen}
            <div role="menu" class="absolute top-full right-0 z-20 mt-1 w-72 rounded-md border border-slate-200 bg-white py-1 font-sans text-sm shadow-lg dark:border-slate-700 dark:bg-slate-900">
              <p class="truncate px-3 py-1 text-xs text-slate-500">{path} — pour {actions.profile()?.key}</p>
              <button type="button" role="menuitem" class="menu-item" onclick={() => run(() => actions.toggleExpand(path))}>
                {actions.isExpanded(path) ? 'Ne plus mettre en avant' : 'Toujours déplier en tête'}
              </button>
              <button type="button" role="menuitem" class="menu-item" onclick={() => run(() => actions.toggleColumn(path))}>
                {actions.isColumn(path) ? 'Retirer la colonne' : 'Afficher en colonne'}
              </button>
              {#each roles as r (r.role)}
                <button type="button" role="menuitem" class="menu-item" onclick={() => run(() => actions.useAs(r.role, actions.roleOf(path) === r.role ? null : path))}>
                  {actions.roleOf(path) === r.role ? '✓ Utilisé comme ' + r.label + ' (annuler)' : 'Utiliser comme ' + r.label}
                </button>
              {/each}
              {#if !isCollection}
                <button type="button" role="menuitem" class="menu-item" onclick={() => run(() => actions.filterOn(value))}>Filtrer sur cette valeur</button>
              {/if}
              <hr class="my-1 border-slate-200 dark:border-slate-700" />
              <button type="button" role="menuitem" class="menu-item" onclick={() => run(copyPath)}>Copier le chemin</button>
            </div>
          {/if}
        {:else if path}
          <button type="button" class="detail-action" title="Copier le chemin" onclick={copyPath}>{'{}'}</button>
        {/if}
      </span>
    </div>
  {/if}

  {#if multiline && actions?.isExpanded(path)}
    <span class="ml-4 text-slate-400 italic">affiché en tête</span>
  {:else if multiline}
    <pre
      class="my-1 ml-4 max-h-96 overflow-auto rounded border border-slate-200 bg-slate-50 p-2 whitespace-pre-wrap wrap-anywhere [tab-size:4] text-slate-800 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200">{#each split(value as string, highlight) as seg, i (i)}<span
          class:hl={seg.match}>{seg.text}</span
        >{/each}</pre>
  {/if}

  {#if isCollection && expanded}
    <div class={isRoot ? '' : 'ml-3 border-l border-slate-200 pl-1 dark:border-slate-700'}>
      {#each entries as e (e.key)}
        <Self key={e.key} value={e.value} path={childPath(e.key)} depth={depth + 1} {filter} {highlight} />
      {/each}
    </div>
  {/if}
</div>
