<script lang="ts">
  import { untrack } from 'svelte';
  import type { AgentAction, AgentActionQuery, ServiceId } from '../../../shared/api';
  import { copy } from '../core/clipboard';
  import { notifications } from '../core/notification.svelte';
  import { actionsService as actions } from './actions.service';

  /** Page « Actions de l'agent » : écritures faites par les agents via le MCP. */
  const SERVICES: { id: ServiceId; label: string }[] = [
    { id: 'gitlab', label: 'GitLab' },
    { id: 'jira', label: 'Jira' },
    { id: 'confluence', label: 'Confluence' },
    { id: 'rocketchat', label: 'Rocket.Chat' },
  ];
  const PAGE_SIZE = 100;

  let entries = $state.raw<AgentAction[]>([]);
  let total = $state(0);
  let query = $state.raw<AgentActionQuery>({});
  const filtered = $derived(!!(query.service || query.status || query.text));

  actions.markSeen();
  $effect(() => {
    actions.last(); // nouvelle action : liste relue
    const q = query;
    untrack(() => {
      void load(q, PAGE_SIZE);
      actions.markSeen();
    });
  });

  async function load(q: AgentActionQuery, limit: number) {
    try {
      const result = await window.ocekour?.actions.list({ ...q, offset: 0, limit });
      if (!result) return;
      entries = result.entries;
      total = result.total;
    } catch (e) {
      notifications.error(e);
    }
  }

  const setQuery = (patch: AgentActionQuery) => (query = { ...query, ...patch });
  const label = (id: ServiceId) => SERVICES.find((s) => s.id === id)?.label ?? id;
  const when = (date: string) => new Date(date).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'medium' });

  function open(event: Event, url: string) {
    event.preventDefault();
    event.stopPropagation();
    void window.ocekour?.actions.openUrl(url).catch((e) => notifications.error(e));
  }
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex flex-wrap items-center gap-2 border-b border-slate-200 px-4 py-2 text-sm dark:border-slate-800">
    <h2 class="mr-2 font-semibold">Actions de l'agent</h2>
    <select class="input w-36 py-1" aria-label="Service" onchange={(e) => setQuery({ service: (e.currentTarget.value || undefined) as ServiceId | undefined })}>
      <option value="">Tous les services</option>
      {#each SERVICES as s (s.id)}<option value={s.id}>{s.label}</option>{/each}
    </select>
    <select class="input w-32 py-1" aria-label="Statut" onchange={(e) => setQuery({ status: (e.currentTarget.value || undefined) as 'ok' | 'error' | undefined })}>
      <option value="">Tous statuts</option>
      <option value="ok">Réussies</option>
      <option value="error">En échec</option>
    </select>
    <input type="search" class="input w-64 py-1" placeholder="Filtrer (texte, projet, ticket…)" oninput={(e) => setQuery({ text: e.currentTarget.value || undefined })} />
    <span class="ml-auto text-xs text-slate-500">{total} action(s)</span>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#each entries as a (a.id)}
      <details class="border-b border-slate-100 px-4 py-2 text-sm dark:border-slate-800/60">
        <summary class="flex cursor-pointer items-center gap-3">
          <span class="w-36 shrink-0 font-mono text-xs text-slate-500">{when(a.date)}</span>
          <span class="w-4 shrink-0" class:text-emerald-600={a.status === 'ok'} class:text-red-600={a.status === 'error'} title={a.status === 'ok' ? 'Réussie' : 'Échec'}>{a.status === 'ok' ? '✓' : '✗'}</span>
          <span class="w-24 shrink-0 text-slate-500">{label(a.service)}</span>
          <code class="w-56 shrink-0 truncate text-xs">{a.tool}</code>
          <span class="min-w-0 flex-1 truncate">{a.target}</span>
          {#if a.url}<button type="button" class="btn py-0 text-xs" onclick={(e) => open(e, a.url!)}>Ouvrir ↗</button>{/if}
        </summary>
        <div class="mt-2 space-y-1 pl-40 text-xs">
          {#if a.error}<p class="text-red-600">{a.error}</p>{/if}
          <p class="text-slate-500">
            Client : {a.client ?? 'inconnu'}
            {#if a.url}· <span class="font-mono">{a.url}</span>{/if}
          </p>
          <div class="flex items-start gap-2">
            <pre class="max-h-80 flex-1 overflow-auto rounded bg-slate-100 p-2 font-mono whitespace-pre-wrap dark:bg-slate-800">{JSON.stringify(a.params, null, 2)}</pre>
            <button type="button" class="btn py-0 text-xs" onclick={() => copy(JSON.stringify(a, null, 2), 'Action copiée')}>Copier</button>
          </div>
        </div>
      </details>
    {:else}
      <div class="m-auto max-w-md p-10 text-center text-sm text-slate-500">
        <p>Aucune écriture faite par un agent{filtered ? ' pour ces filtres' : ''}.</p>
        <p class="mt-2 text-xs">Les commentaires, tickets, pages et messages créés via le serveur MCP d'Ocekour apparaissent ici, avec un lien vers l'élément.</p>
      </div>
    {/each}
    {#if entries.length < total}
      <div class="p-3 text-center"><button type="button" class="btn" onclick={() => load(query, entries.length + PAGE_SIZE)}>Afficher plus</button></div>
    {/if}
  </div>
</div>
