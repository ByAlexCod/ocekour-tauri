<script lang="ts">
  import { onMount } from 'svelte';
  import type { AgentState, AgentTool, ContextSource, ContextStatus } from '../../../shared/api';
  import { configService } from '../core/config.svelte';
  import { notifications } from '../core/notification.svelte';

  /**
   * Paramètres → Contexte IA : Ocekour tient une mémoire de contexte (messages Rocket.Chat reçus,
   * nouveautés GitLab / Jira / Confluence) exposée aux agents locaux par le MCP. Désactivée par
   * défaut ; à la première activation, part de « maintenant » (aucun rattrapage).
   */
  const AGENT_LABEL: Record<AgentTool, string> = { qwen: 'Qwen Code', vibe: 'Mistral Vibe' };
  const sources: { id: ContextSource; label: string; hint: string }[] = [
    { id: 'rocketchat', label: 'Rocket.Chat', hint: 'messages reçus et envoyés (DM et canaux)' },
    { id: 'gitlab', label: 'GitLab', hint: 'MR, commentaires, revues, pipelines' },
    { id: 'jira', label: 'Jira', hint: 'tickets, commentaires, statuts' },
    { id: 'confluence', label: 'Confluence', hint: 'pages suivies, commentaires' },
  ];

  let status = $state.raw<ContextStatus | undefined>(undefined);
  let agents = $state.raw<AgentState[]>([]);
  /** LLM de curation détecté pour le mode courant (affiché immédiatement au changement). */
  let llm = $state<string | null>(null);
  const c = $derived(configService.config?.contexte);

  $effect(() => {
    void configService.config;
    const api = window.ocekour?.contexte;
    void api?.status().then((s) => (status = s)).catch(() => undefined);
    void api?.detectLlm().then((l) => (llm = l)).catch(() => (llm = null));
    void api?.agents().then((a) => (agents = a)).catch(() => undefined);
  });
  onMount(() => window.ocekour?.contexte.onChanged((s) => (status = s)));

  const patch = (p: Record<string, unknown>) => void window.ocekour?.config.setContexte(p).catch((e) => notifications.error(e));
  const patchTls = (p: { insecure?: boolean; caFile?: string | null }) => patch({ tls: p });
  const caName = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const time = (iso: string) => new Date(iso).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });

  async function pickCa() {
    try {
      const info = await window.ocekour?.connections.pickCaFile();
      if (info) patchTls({ caFile: info.path });
    } catch (e) {
      notifications.error(e);
    }
  }

  function setInterval(input: HTMLInputElement) {
    const value = input.valueAsNumber;
    if (Number.isInteger(value) && value >= 30 && value <= 3_600) patch({ intervalSeconds: value });
  }

  async function configure(tool: AgentTool, action: 'add' | 'remove') {
    try {
      const next = await window.ocekour?.contexte.configureAgent(tool, action);
      if (next) agents = next;
      notifications.info(action === 'add' ? `${AGENT_LABEL[tool]} : serveur MCP Ocekour enregistré` : `${AGENT_LABEL[tool]} : serveur MCP Ocekour retiré`);
    } catch (e) {
      notifications.error(e);
    }
  }
</script>

{#if c}
  <div class="space-y-3">
    <label class="flex items-center gap-1.5">
      <input type="checkbox" checked={c.enabled} onchange={(e) => patch({ enabled: e.currentTarget.checked })} />
      Nourrir une mémoire de contexte et l'exposer à mon agent (Qwen Code / Mistral Vibe)
    </label>
    <p class="text-xs text-slate-500">
      Chaque message Rocket.Chat reçu et chaque nouveauté GitLab / Jira / Confluence alimentent un contexte local. L'agent y accède par les
      outils MCP <code>contexte_*</code>. À la première activation, on part de maintenant (pas de rattrapage). Rien ne quitte le poste.
    </p>

    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-slate-500">
      <label class="flex items-center gap-1.5">
        Toutes les
        <input type="number" class="input w-20 py-0.5" min="30" max="3600" step="30" value={c.intervalSeconds} onchange={(e) => setInterval(e.currentTarget)} />
        secondes
      </label>
      <label class="flex items-center gap-1.5">
        Résumé par le LLM de
        <select class="input py-0.5" value={c.llm} onchange={(e) => patch({ llm: e.currentTarget.value })}>
          <option value="auto">Qwen ou Vibe (auto)</option>
          <option value="qwen">Qwen Code</option>
          <option value="vibe">Mistral Vibe</option>
          <option value="off">Aucun (texte brut)</option>
        </select>
      </label>
      {#if llm}
        <span class="text-emerald-600">● {llm}</span>
      {:else if c.llm !== 'off'}
        <span
          class="text-amber-600"
          title="Config Qwen/Vibe introuvable. Qwen : ~/.qwen/settings.json ou ~/.qwen/.env (OPENAI_BASE_URL, OPENAI_MODEL). Sans LLM, le contexte est stocké en texte brut."
          >LLM introuvable — stockage brut</span
        >
      {/if}
    </div>

    {#if c.llm !== 'off'}
      <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
        <span>TLS de l'endpoint LLM :</span>
        <label class="flex items-center gap-1.5">
          <input type="checkbox" checked={c.tls.insecure} onchange={(e) => patchTls({ insecure: e.currentTarget.checked })} />
          Ignorer le certificat (https auto-signé)
        </label>
        {#if c.tls.caFile}
          <span class="font-mono" title={c.tls.caFile}>CA : {caName(c.tls.caFile)}</span>
          <button type="button" class="btn py-0.5" onclick={() => patchTls({ caFile: null })}>Retirer</button>
        {:else}
          <button type="button" class="btn py-0.5" onclick={pickCa}>Certificat d'autorité…</button>
        {/if}
      </div>
    {/if}

    <div class="space-y-1">
      {#each sources as row (row.id)}
        <label class="flex items-center gap-1.5 text-sm">
          <input type="checkbox" checked={c.sources[row.id].enabled} onchange={(e) => patch({ sources: { [row.id]: { enabled: e.currentTarget.checked } } })} />
          {row.label}
          <span class="text-xs text-slate-500">— {row.hint}</span>
        </label>
      {/each}
    </div>

    <div class="text-xs">
      {#if status?.lastError}
        <span class="text-red-600">{status.lastError}</span>
      {:else if status?.running}
        <span class="text-emerald-600">● actif</span>
        <span class="text-slate-500">
          · {status.entities} entité(s)
          {#if status.lastRun}
            · dernier passage {time(status.lastRun)}{#if status.lastCount}, {status.lastCount} ajoutée(s){/if}
          {/if}
        </span>
      {:else if c.enabled}
        <span class="text-slate-500">démarrage…</span>
      {:else}
        <span class="text-slate-500">arrêté</span>
      {/if}
    </div>

    <div class="space-y-1">
      <span class="text-sm font-medium">Configurer mon agent</span>
      <p class="text-xs text-slate-500">
        Enregistre le serveur MCP d'Ocekour dans la configuration de l'agent (sauvegarde <code>.bak</code> du fichier). Le serveur MCP doit
        être activé (voir ci-dessous).
      </p>
      {#each agents as a (a.tool)}
        <div class="flex flex-wrap items-center gap-2 text-xs">
          <span class="w-24">{AGENT_LABEL[a.tool]}</span>
          {#if !a.present}
            <span class="text-slate-500">non détecté ({a.file})</span>
          {:else if a.configured}
            <span class="text-emerald-600">● branché</span>
            <button type="button" class="btn py-0.5" onclick={() => configure(a.tool, 'remove')}>Retirer</button>
          {:else}
            <button type="button" class="btn py-0.5" onclick={() => configure(a.tool, 'add')}>Configurer</button>
          {/if}
        </div>
      {/each}
    </div>
  </div>
{/if}
