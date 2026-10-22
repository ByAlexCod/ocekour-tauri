<script lang="ts">
  import type { McpStatus } from '../../../shared/api';
  import { copy } from '../core/clipboard';
  import { configService } from '../core/config.svelte';
  import { type ClientId, MCP_CLIENTS, mcpSnippets } from './mcp-snippets';

  /**
   * Serveur MCP intégré : un agent (Claude Code, Qwen Code…) utilise Ocekour comme l'interface
   * (connexions, logs, Kafka, S3…). HTTP sur 127.0.0.1 avec jeton, ou sans interface
   * (`--mcp`, `--mcp-http`).
   */
  let status = $state.raw<McpStatus | undefined>(undefined);
  let showToken = $state(false);
  /** Client MCP choisi dans l'aide (les formats de configuration diffèrent). */
  let client = $state<ClientId>('claude');
  const config = $derived(configService.config);
  const snippets = $derived(status ? mcpSnippets(status, client) : []);

  // Relu à chaque changement de configuration (case, port).
  $effect(() => {
    void configService.config;
    void window.ocekour?.mcp
      .status()
      .then((s) => (status = s))
      .catch(() => undefined);
  });

  function setPort(input: HTMLInputElement) {
    const port = input.valueAsNumber;
    if (Number.isInteger(port) && port >= 1024 && port <= 65535) configService.setPreferences({ mcpPort: port });
  }

  async function regenerate() {
    if (!confirm("Nouveau jeton : les clients MCP configurés avec l'ancien ne fonctionneront plus.")) return;
    const s = await window.ocekour?.mcp.regenerateToken();
    if (s) status = s;
  }
</script>

<div class="space-y-2">
  <label class="flex items-center gap-1.5">
    <input type="checkbox" checked={config?.mcpServer} onchange={(e) => configService.setPreferences({ mcpServer: e.currentTarget.checked })} />
    Activer le serveur MCP pendant que l'interface est ouverte
  </label>
  <div class="flex flex-wrap items-center gap-2 text-xs text-slate-500">
    <label class="flex items-center gap-1.5">
      Port
      <input type="number" class="input w-24 py-0.5" min="1024" max="65535" value={config?.mcpPort} onchange={(e) => setPort(e.currentTarget)} />
    </label>
    {#if status?.running}
      <span class="text-emerald-600">● actif sur <code>{status.url}</code></span>
    {:else if status?.error}
      <span class="text-red-600">{status.error}</span>
    {:else}
      <span>arrêté</span>
    {/if}
  </div>

  {#if status}
    {@const s = status}
    <div class="flex flex-wrap items-center gap-2 text-xs">
      <span class="text-slate-500">Jeton</span>
      <code class="rounded bg-slate-100 px-1.5 py-0.5 dark:bg-slate-800">{showToken ? s.token : '••••••••••••'}</code>
      <button type="button" class="btn py-0 text-xs" onclick={() => (showToken = !showToken)}>{showToken ? 'Masquer' : 'Afficher'}</button>
      <button type="button" class="btn py-0 text-xs" onclick={() => copy(s.token, 'Jeton copié')}>Copier</button>
      <button type="button" class="btn py-0 text-xs" onclick={regenerate}>Nouveau jeton</button>
    </div>

    <details class="text-xs">
      <summary class="cursor-pointer text-sky-700 dark:text-sky-400">Configurer un client MCP</summary>
      <div class="mt-2 flex w-fit rounded-md border border-slate-300 p-0.5 dark:border-slate-700" role="radiogroup" aria-label="Client MCP">
        {#each MCP_CLIENTS as c (c.id)}
          <button type="button" role="radio" class="seg text-xs" aria-checked={client === c.id} onclick={() => (client = c.id)}>{c.label}</button>
        {/each}
      </div>
      <div class="mt-2 space-y-3">
        {#each snippets as snippet (snippet.title)}
          <div>
            <div class="flex items-center gap-2">
              <strong>{snippet.title}</strong>
              <button type="button" class="btn py-0 text-xs" onclick={() => copy(snippet.text, 'Configuration copiée')}>Copier</button>
            </div>
            {#if snippet.note}<p class="mt-0.5 text-slate-500">{snippet.note}</p>{/if}
            <pre class="mt-1 overflow-x-auto rounded bg-slate-100 p-2 font-mono dark:bg-slate-800">{snippet.text}</pre>
          </div>
        {/each}
      </div>
    </details>
    <p class="text-xs text-slate-500">
      Le serveur n'écoute que ce poste (127.0.0.1) et exige le jeton. Les outils font les mêmes actions que l'interface ; les secrets ne sont
      jamais renvoyés.
    </p>
  {/if}
</div>
