<script lang="ts">
  import { untrack } from 'svelte';
  import type { CaFileInfo, ServiceAccount, ServiceTestResult } from '../../../shared/api';
  import TlsField from '../connections/TlsField.svelte';
  import { notifications } from '../core/notification.svelte';
  import { accountDirty, HELP, PLACEHOLDER, WRITE_HINT } from './services-help';

  let { account: a, onchanged }: { account: ServiceAccount; onchanged: (list: ServiceAccount[]) => void } = $props();

  let url = $state('');
  let insecure = $state(false);
  let ca = $state.raw<CaFileInfo | null>(null);
  let pasting = $state(false);
  let token = $state('');
  let userId = $state('');
  let busy = $state(false);
  let result = $state.raw<ServiceTestResult | undefined>(undefined);
  let tested: string | undefined;

  const help = $derived(HELP[a.id]);
  const dirty = $derived(accountDirty(a, { url, insecure, caFile: ca?.path }));

  // Valeurs affichées = configuration (rechargée après chaque enregistrement).
  $effect(() => {
    const account = a;
    untrack(() => {
      url = account.url ?? '';
      insecure = account.tls.insecure;
      const caFile = account.tls.caFile;
      if (!caFile) ca = null;
      else
        window.ocekour?.connections
          .readCaFile(caFile)
          .then((info) => (ca = info))
          .catch(() => (ca = null));
      // Jeton déjà enregistré : testé une fois à l'affichage.
      const key = `${account.url}|${account.hasToken}`;
      if (account.url && account.hasToken && tested !== key) {
        tested = key;
        void test();
      }
      if (!account.hasToken) result = undefined;
    });
  });

  async function run<T>(action: () => Promise<T> | undefined): Promise<T | undefined> {
    busy = true;
    try {
      return await action();
    } catch (e) {
      notifications.error(e);
      return undefined;
    } finally {
      busy = false;
    }
  }

  async function saveUrl() {
    const list = await run(() => window.ocekour?.services.save(a.id, { url, tls: { insecure, ...(ca ? { caFile: ca.path } : {}) } }));
    if (list) onchanged(list);
  }

  async function saveToken(event: SubmitEvent) {
    event.preventDefault();
    const r = await run(() => window.ocekour?.services.setToken(a.id, token, userId || undefined));
    if (!r) return;
    result = r;
    token = '';
    userId = '';
    pasting = false;
    tested = `${a.url}|true`;
    if (r.ok) notifications.info(`Jeton ${a.label} enregistré`);
    const list = await window.ocekour?.services.list();
    if (list) onchanged(list);
  }

  async function test() {
    const r = await run(() => window.ocekour?.services.test(a.id));
    if (r) result = r;
  }

  async function forget() {
    if (!confirm(`Oublier le jeton ${a.label} ? Les agents n'y auront plus accès.`)) return;
    const list = await run(() => window.ocekour?.services.forgetToken(a.id));
    if (list) onchanged(list);
  }

  async function setFlags(flags: { enabled?: boolean; write?: boolean }) {
    const list = await run(() => window.ocekour?.services.setFlags(a.id, flags));
    if (list) onchanged(list);
  }

  const openTokenPage = (fallback: boolean) => void run(() => window.ocekour?.services.openTokenPage(a.id, fallback));
</script>

<section class="rounded-lg border border-slate-200 p-4 dark:border-slate-800">
  <header class="mb-3 flex items-center gap-2">
    <h3 class="font-semibold">{a.label}</h3>
    {#if !a.url}
      <span class="text-xs text-slate-500">non configuré</span>
    {:else if a.hasToken && a.enabled}
      <span class="text-xs text-emerald-600">● disponible pour les agents</span>
    {:else if !a.enabled}
      <span class="text-xs text-slate-500">désactivé</span>
    {:else}
      <span class="text-xs text-amber-600">jeton manquant</span>
    {/if}
  </header>

  <div class="grid grid-cols-[7rem_1fr] items-start gap-x-4 gap-y-3 text-sm">
    <label class="pt-1.5 font-medium" for="{a.id}-url">URL</label>
    <div class="space-y-2">
      <div class="flex gap-2">
        <input id="{a.id}-url" type="url" class="input flex-1 font-mono" placeholder={PLACEHOLDER[a.id]} bind:value={url} />
        <button type="button" class="btn" disabled={!dirty} onclick={saveUrl}>Enregistrer</button>
      </div>
      {#if url}
        <details class="text-xs" open={insecure || !!ca}>
          <summary class="cursor-pointer text-slate-500">Certificat (TLS)</summary>
          <div class="mt-2"><TlsField name="{a.id}-tls" bind:insecure bind:ca /></div>
        </details>
      {/if}
    </div>

    {#if a.url}
      <span class="pt-1 font-medium">Jeton</span>
      <div class="space-y-2">
        <div class="flex flex-wrap items-center gap-2">
          {#if a.hasToken}
            <code class="rounded bg-slate-100 px-1.5 py-0.5 dark:bg-slate-800">••••••••</code>
            {#if result}
              {#if result.ok}
                <span class="text-emerald-600">✓ {result.identity}</span>
              {:else}
                <span class="text-red-600">{result.error}</span>
              {/if}
            {/if}
          {:else}
            <span class="text-slate-500">aucun</span>
          {/if}
        </div>
        {#if result?.warning}<p class="text-xs text-amber-600">{result.warning}</p>{/if}
        <div class="flex flex-wrap gap-2">
          <button type="button" class="btn" onclick={() => openTokenPage(false)}>Créer un jeton ↗</button>
          {#if a.id === 'gitlab'}
            <button type="button" class="btn text-xs" onclick={() => openTokenPage(true)}>ancienne page ↗</button>
          {/if}
          <button type="button" class="btn" onclick={() => (pasting = !pasting)}>{a.hasToken ? 'Remplacer le jeton' : 'Coller le jeton'}</button>
          {#if a.hasToken}
            <button type="button" class="btn" disabled={busy} onclick={test}>Tester</button>
            <button type="button" class="btn" onclick={forget}>Oublier</button>
          {/if}
        </div>
        {#if pasting}
          <form class="flex flex-wrap items-center gap-2" onsubmit={saveToken}>
            {#if a.needsUserId}
              <input type="text" class="input w-56 font-mono" placeholder="Identifiant utilisateur" autocomplete="off" bind:value={userId} />
            {/if}
            <input type="password" class="input w-72 font-mono" placeholder="Jeton" autocomplete="off" bind:value={token} />
            <button type="submit" class="btn btn-primary" disabled={busy || !token || (a.needsUserId && !userId)}>Enregistrer</button>
          </form>
        {/if}
        <details class="text-xs">
          <summary class="cursor-pointer text-sky-700 dark:text-sky-400">Comment créer le jeton ?</summary>
          <ol class="mt-1 list-decimal space-y-0.5 pl-5 text-slate-600 dark:text-slate-400">
            {#each help.steps as step, i (i)}<li>{step}</li>{/each}
          </ol>
          {#if help.note}<p class="mt-1 text-slate-500">{help.note}</p>{/if}
          <p class="mt-1 text-slate-500">Le jeton est chiffré sur ce poste (Windows) ; il n'est jamais écrit dans config.yaml ni transmis aux agents.</p>
        </details>
      </div>

      <span class="font-medium">Agents</span>
      <div class="space-y-1">
        <label class="flex items-center gap-1.5">
          <input type="checkbox" checked={a.enabled} onchange={(e) => setFlags({ enabled: e.currentTarget.checked })} />
          Outils {a.label} visibles par les agents
        </label>
        <label class="flex items-center gap-1.5">
          <input type="checkbox" checked={a.write} disabled={!a.enabled} onchange={(e) => setFlags({ write: e.currentTarget.checked })} />
          Autoriser l'écriture
          <span class="text-xs text-slate-500">({WRITE_HINT[a.id]})</span>
        </label>
      </div>
    {/if}
  </div>
</section>
