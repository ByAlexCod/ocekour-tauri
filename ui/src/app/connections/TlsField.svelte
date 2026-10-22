<script lang="ts">
  import type { CaFileInfo } from '../../../shared/api';
  import { notifications } from '../core/notification.svelte';

  /** Choix TLS : autorités du système (+ fichier .cer facultatif) ou vérification ignorée. */
  let {
    name,
    insecure = $bindable(false),
    ca = $bindable(null),
  }: { name: string; insecure?: boolean; ca?: CaFileInfo | null } = $props();

  async function pick() {
    try {
      const info = await window.ocekour?.connections.pickCaFile();
      if (info) ca = info;
    } catch (e) {
      notifications.error(e);
    }
  }
</script>

<div class="space-y-2">
  <label class="flex items-start gap-2">
    <input type="radio" class="mt-0.5" {name} checked={!insecure} onchange={() => (insecure = false)} />
    <span>
      Vérifier le certificat
      <span class="block text-xs text-slate-500">Les autorités installées dans Windows sont reconnues automatiquement.</span>
    </span>
  </label>
  {#if !insecure}
    <div class="ml-6 flex flex-wrap items-center gap-2">
      {#if ca}
        <span class="min-w-0 flex-1 truncate rounded bg-slate-100 px-2 py-1 font-mono text-xs dark:bg-slate-800" title={ca.path}>
          {ca.subject} · expire le {ca.validTo.slice(0, 10)}
          {#if ca.expired}<strong class="text-red-600"> (expiré)</strong>{/if}
        </span>
        <button type="button" class="btn" onclick={() => (ca = null)}>Retirer</button>
      {:else}
        <button type="button" class="btn" onclick={pick}>Ajouter un certificat d'autorité (.cer)…</button>
      {/if}
    </div>
  {/if}
  <label class="flex items-center gap-2">
    <input type="radio" {name} checked={insecure} onchange={() => (insecure = true)} />
    Ignorer la vérification SSL <span class="text-xs text-amber-600">(déconseillé)</span>
  </label>
</div>
