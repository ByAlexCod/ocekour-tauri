<script lang="ts">
  import type { KafkaMeta } from '../../../../shared/api';
  import { copy } from '../../core/clipboard';
  import { fieldActions } from '../field-actions';
  import type { Matcher } from '../filter';
  import type { LogEntry } from '../log-buffer';
  import { expandNested, getFieldDeep } from '../parser';
  import JsonNode from './JsonNode.svelte';

  const FORMAT_LABELS: Record<KafkaMeta['format'], string> = {
    avro: 'Avro',
    json: 'JSON',
    text: 'texte',
    hex: 'binaire (hexadécimal)',
    null: 'valeur vide',
  };

  /** Détail d'une ligne : champs mis en avant, arbre JSON lisible, ligne brute. */
  let {
    entry,
    expand = [],
    parseNestedDefault = false,
    highlight,
    onclose,
  }: {
    entry: LogEntry;
    /** Champs à afficher en tête, dépliés (profil du workload). */
    expand?: string[];
    /** Désérialiser le JSON contenu dans des chaînes (valeur par défaut du profil). */
    parseNestedDefault?: boolean;
    /** Recherche en cours dans le flux, surlignée dans le détail. */
    highlight?: Matcher;
    onclose: () => void;
  } = $props();

  const actions = fieldActions();
  let tab = $state<'fields' | 'raw'>('fields');
  let filter = $state('');
  let nestedChoice = $state<boolean | undefined>(undefined);
  $effect(() => {
    void parseNestedDefault;
    nestedChoice = undefined;
  });
  const parseNested = $derived(nestedChoice ?? parseNestedDefault);

  const headers = $derived(Object.entries(entry.meta?.headers ?? {}).sort(([a], [b]) => a.localeCompare(b)));
  const json = $derived.by(() => {
    const j = entry.parsed.json;
    if (!j) return undefined;
    return parseNested ? (expandNested(j) as Record<string, unknown>) : j;
  });
  /** Champs du profil présents dans la ligne, affichés en premier. */
  const highlighted = $derived(json ? expand.map((path) => ({ path, value: getFieldDeep(json, path) })).filter((f) => f.value !== undefined) : []);
  const pretty = $derived(entry.parsed.json ? JSON.stringify(json, null, 2) : entry.text);
  const display = (value: unknown) => (typeof value === 'string' ? value : JSON.stringify(value, null, 2));
</script>

<div class="flex items-center gap-2 border-b border-slate-200 px-3 py-1.5 text-sm dark:border-slate-800">
  <div class="flex gap-3" role="tablist">
    <button type="button" role="tab" class="detail-tab" aria-selected={tab === 'fields'} onclick={() => (tab = 'fields')}>Détail</button>
    <button type="button" role="tab" class="detail-tab" aria-selected={tab === 'raw'} onclick={() => (tab = 'raw')}>Brut</button>
  </div>
  <span class="flex-1"></span>
  <button type="button" class="btn py-0.5" onclick={() => copy(pretty, 'JSON formaté copié')}>Copier {entry.parsed.json ? 'le JSON' : 'le texte'}</button>
  <button type="button" class="btn py-0.5" title="Copier la ligne brute" onclick={() => copy(entry.text, 'Ligne brute copiée')}>Brut</button>
  <button type="button" class="btn px-2 py-0.5" aria-label="Fermer le détail (Échap)" onclick={onclose}>✕</button>
</div>

{#if tab === 'raw'}
  <pre class="min-h-0 flex-1 overflow-auto p-3 font-mono text-xs break-all whitespace-pre-wrap wrap-anywhere [tab-size:4]">{entry.text}</pre>
{:else}
  <div class="min-h-0 flex-1 overflow-auto p-3 font-mono text-xs">
    <div class="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1 font-sans text-sm text-slate-500">
      {#if entry.parsed.time}<span>{entry.parsed.time.toLocaleString('fr-FR')}</span>{/if}
      {#if entry.parsed.level !== 'unknown'}<span class="font-semibold uppercase">{entry.parsed.level}</span>{/if}
      {#if entry.meta}
        <span>partition {entry.meta.partition} · offset {entry.meta.offset}</span>
        <span class="rounded bg-slate-100 px-1.5 text-xs dark:bg-slate-800">{FORMAT_LABELS[entry.meta.format]}</span>
      {/if}
    </div>

    {#if entry.meta}
      {@const m = entry.meta}
      {#if m.warning}
        <p role="note" class="mb-3 rounded border border-amber-300 bg-amber-50 px-2 py-1 font-sans text-sm text-amber-800 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200">{m.warning}</p>
      {/if}
      <dl class="mb-3 grid grid-cols-[max-content_1fr] gap-x-3 gap-y-0.5 border-b border-slate-200 pb-3 dark:border-slate-800" aria-label="Message Kafka">
        <dt class="text-slate-500">clé</dt>
        <dd class="break-all">
          {#if m.key !== undefined}{m.key}{:else}<span class="text-slate-400 italic">aucune</span>{/if}
        </dd>
        {#each headers as h (h[0])}
          <dt class="text-slate-500" title={'Header ' + h[0]}>{h[0]}</dt>
          <dd class="break-all">{h[1]}</dd>
        {/each}
      </dl>
    {/if}

    {#if json}
      <div class="mb-3 flex items-center gap-2 font-sans text-sm">
        <input type="search" class="input min-w-0 flex-1" placeholder="Rechercher un champ ou une valeur…" aria-label="Rechercher dans le détail" bind:value={filter} />
        <label class="flex shrink-0 items-center gap-1.5" title="Afficher en arbre le JSON contenu dans des chaînes">
          <input type="checkbox" checked={parseNested} onchange={(e) => (nestedChoice = e.currentTarget.checked)} />
          JSON imbriqué
        </label>
        {#if actions && parseNested !== parseNestedDefault}
          <button type="button" class="shrink-0 text-xs text-sky-600 hover:underline" title="Enregistrer ce choix dans le profil du workload" onclick={() => actions.setParseNested(parseNested)}
            >Toujours pour ce workload</button
          >
        {/if}
      </div>

      {#each highlighted as field (field.path)}
        <section class="mb-3">
          <div class="mb-1 flex items-center gap-2 font-sans text-sm font-semibold text-violet-700 dark:text-violet-300">
            {field.path}
            <button type="button" class="detail-action" title="Copier la valeur" onclick={() => copy(display(field.value), 'Valeur copiée')}>⧉</button>
            {#if actions}
              <button type="button" class="detail-action" title="Ne plus mettre ce champ en avant pour ce workload" onclick={() => actions.toggleExpand(field.path)}>✕</button>
            {/if}
          </div>
          <pre class="max-h-[50vh] overflow-auto rounded border border-slate-200 bg-slate-50 p-2 whitespace-pre-wrap wrap-anywhere [tab-size:4] dark:border-slate-700 dark:bg-slate-900">{display(field.value)}</pre>
        </section>
      {/each}

      <JsonNode value={json} filter={filter.trim().toLowerCase()} {highlight} />
    {:else}
      <pre class="break-all whitespace-pre-wrap wrap-anywhere [tab-size:4]">{entry.text}</pre>
    {/if}
  </div>
{/if}
