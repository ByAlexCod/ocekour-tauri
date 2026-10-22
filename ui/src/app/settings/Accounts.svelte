<script lang="ts">
  import type { ServiceAccount } from '../../../shared/api';
  import { configService } from '../core/config.svelte';
  import ServiceCard from './ServiceCard.svelte';

  /** Paramètres → Comptes : GitLab, Jira, Confluence, Rocket.Chat pour les agents (MCP). */
  let { onshowactions }: { onshowactions?: () => void } = $props();
  let accounts = $state.raw<ServiceAccount[]>([]);

  // Rechargé si config.yaml change (clé d'activation, édition du fichier).
  $effect(() => {
    void configService.config;
    void window.ocekour?.services
      .list()
      .then((list) => (accounts = list))
      .catch(() => undefined);
  });
</script>

<div class="max-w-3xl space-y-4 p-6 text-sm">
  <p class="text-slate-600 dark:text-slate-400">
    Donnez aux agents (Claude Code, Qwen Code, Mistral Vibe…) accès à vos outils via le serveur MCP d'Ocekour, avec <strong>vos</strong>
    droits : chaque jeton est créé par vous, sur votre compte. Les écritures sont signées « via Ocekour » et listées dans
    <button type="button" class="text-sky-700 underline dark:text-sky-400" onclick={() => onshowactions?.()}>Actions de l'agent</button>.
  </p>
  {#each accounts as a (a.id)}
    <ServiceCard account={a} onchanged={(list) => (accounts = list)} />
  {/each}
</div>
