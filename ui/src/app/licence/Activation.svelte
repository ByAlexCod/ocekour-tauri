<script lang="ts" module>
  export const MODE_LABELS = {
    none: 'Sans suivi',
    optional: 'Suivi si possible',
    required: 'Suivi obligatoire',
  } as const;
</script>

<script lang="ts">
  import { licence } from '../core/licence.svelte';

  /**
   * Écran plein qui recouvre l'application tant qu'elle n'est pas utilisable : activation
   * (clé à coller), vérification, ou blocage du suivi obligatoire (Matomo injoignable).
   */
  let busy = $state(false);
  let error = $state<string | undefined>(undefined);
  let key = $state('');
  const current = $derived(licence.state);
  const view = $derived(licence.changing ? 'form' : current.status === 'missing' ? 'form' : current.status);
  const title = $derived(
    view === 'checking'
      ? 'Vérification de la clé…'
      : view === 'blocked'
        ? 'Serveur de statistiques injoignable'
        : licence.changing
          ? 'Changer de clé d’activation'
          : 'Activer Ocekour',
  );
  const modeLabel = $derived(current.info?.mode ? MODE_LABELS[current.info.mode] : '');

  async function activate() {
    if (!key.trim()) return;
    busy = true;
    error = await licence.activate(key);
    busy = false;
  }

  async function retry() {
    busy = true;
    await licence.retry();
    busy = false;
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-slate-100/95 p-6 dark:bg-slate-950/95" role="dialog" aria-modal="true" aria-labelledby="activation-title">
  <div class="w-full max-w-xl rounded-xl border border-slate-200 bg-white p-6 shadow-xl dark:border-slate-800 dark:bg-slate-900">
    <div class="mb-4 flex items-center gap-3">
      <img src="icon.png" alt="" class="size-10" />
      <h1 id="activation-title" class="text-lg font-semibold">{title}</h1>
    </div>

    {#if view === 'checking'}
      <p class="text-sm text-slate-600 dark:text-slate-300">Connexion au serveur de statistiques de l'équipe…</p>
    {:else if view === 'blocked'}
      <p class="text-sm text-slate-700 dark:text-slate-200">{current.message}</p>
      <p class="mt-2 text-xs text-slate-500">Clé « {current.info?.label} » · {modeLabel} · {current.info?.id}</p>
      <div class="mt-5 flex gap-2">
        <button type="button" class="btn-primary" disabled={busy} onclick={retry}>{busy ? 'Vérification…' : 'Réessayer'}</button>
        <button type="button" class="btn" onclick={() => (licence.changing = true)}>Changer de clé</button>
      </div>
    {:else}
      <p class="text-sm text-slate-600 dark:text-slate-300">
        Collez la clé d'activation qui vous a été remise (elle commence par <code>OCEKOUR1-</code>).
      </p>
      {#if current.message && !licence.changing}
        <p class="mt-2 text-sm text-amber-700 dark:text-amber-300">{current.message}</p>
      {/if}
      <textarea
        class="input mt-3 h-28 w-full resize-none font-mono text-xs break-all"
        aria-label="Clé d'activation"
        placeholder="OCEKOUR1-…"
        spellcheck="false"
        bind:value={key}
        oninput={() => (error = undefined)}
        onkeydown={(e) => e.ctrlKey && e.key === 'Enter' && activate()}
      ></textarea>
      {#if error}
        <p role="alert" class="mt-2 text-sm text-red-600 dark:text-red-400">{error}</p>
      {/if}
      <div class="mt-4 flex items-center gap-2">
        <button type="button" class="btn-primary" disabled={busy} onclick={activate}>{busy ? 'Vérification…' : 'Activer'}</button>
        {#if licence.changing}
          <button type="button" class="btn" onclick={() => (licence.changing = false)}>Annuler</button>
        {/if}
        <span class="flex-1"></span>
        {#if current.development}
          <span class="text-xs text-amber-600" title="Accepte aussi les clés de développement">Build de développement</span>
        {/if}
      </div>
      <p class="mt-4 text-xs text-slate-500">
        La clé est enregistrée chiffrée sur ce poste. Selon son type, Ocekour envoie des statistiques d'utilisation anonymes (jamais de
        données de vos sources).
      </p>
    {/if}
  </div>
</div>
