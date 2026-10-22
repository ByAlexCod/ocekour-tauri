<script lang="ts">
  import { configService } from '../core/config.svelte';
  import { notifications } from '../core/notification.svelte';

  /**
   * Fichiers du poste que les agents (MCP) peuvent lire : aucun par défaut. Chemins absolus,
   * fichiers ou motifs (`C:\app\logs\*.log`) ; tout le reste du disque leur reste fermé.
   */
  const agents = $derived(configService.config?.fichiers?.agents ?? []);
  let draft = $state('');

  async function save(next: string[]) {
    try {
      await window.ocekour?.config.setFichiers({ agents: next });
      return true;
    } catch (e) {
      notifications.error(e);
      return false;
    }
  }

  async function add(event: SubmitEvent) {
    event.preventDefault();
    const path = draft.trim();
    if (!path || agents.includes(path)) return;
    if (await save([...agents, path])) draft = '';
  }

  async function addPicked() {
    const picked = (await window.ocekour?.files.pick()) ?? [];
    const next = [...agents, ...picked.filter((p) => !agents.includes(p))];
    if (next.length !== agents.length) await save(next);
  }
</script>

<div class="space-y-2">
  <p class="text-xs text-slate-500">
    Fichiers de logs du poste que les agents peuvent lire (outils <code>fichiers_lister</code>, <code>logs_lire</code>). Aucun par défaut : le
    reste du disque leur reste fermé. Un motif (<code>C:\app\logs\*.log</code>) ouvre les fichiers correspondants de ce dossier.
  </p>
  {#if agents.length}
    <ul class="space-y-1">
      {#each agents as path (path)}
        <li class="flex items-center gap-2 text-xs">
          <code class="min-w-0 flex-1 truncate rounded bg-slate-100 px-1.5 py-0.5 dark:bg-slate-800" title={path}>{path}</code>
          <button type="button" class="btn py-0 text-xs" onclick={() => save(agents.filter((p) => p !== path))}>Retirer</button>
        </li>
      {/each}
    </ul>
  {/if}
  <form class="flex flex-wrap items-center gap-2" onsubmit={add}>
    <input class="input min-w-64 flex-1 py-0.5 font-mono text-xs" placeholder="C:\app\logs\*.log" aria-label="Fichier ou motif" bind:value={draft} />
    <button type="submit" class="btn py-0.5 text-xs" disabled={!draft.trim()}>Ajouter</button>
    <button type="button" class="btn py-0.5 text-xs" onclick={addPicked}>Choisir des fichiers…</button>
  </form>
</div>
