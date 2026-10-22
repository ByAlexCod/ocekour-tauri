<script lang="ts">
  import type { CaFileInfo, DatabaseDraft, DatabaseEngine, DatabaseInfo, DatabaseTestResult } from '../../../shared/api';
  import TlsField from '../connections/TlsField.svelte';
  import { configService } from '../core/config.svelte';
  import { notifications } from '../core/notification.svelte';
  import { canSubmit, defaultPort, ENGINES, engineLabel, toDraft } from './database-draft';

  /** Paramètres → Bases de données : connexions en lecture seule pour les agents (outils MCP bdd_*). */
  let databases = $state.raw<DatabaseInfo[]>([]);
  let engine = $state<DatabaseEngine>('postgres');
  let name = $state('');
  let host = $state('');
  let port = $state('');
  let database = $state('');
  let user = $state('');
  let password = $state('');
  let ssl = $state(false);
  let oracleSid = $state(false);
  let insecure = $state(false);
  let ca = $state.raw<CaFileInfo | null>(null);
  let editingId = $state<string | undefined>(undefined);
  let busy = $state(false);
  let result = $state.raw<DatabaseTestResult | undefined>(undefined);

  const form = $derived({ id: editingId, engine, name, host, port, database, user, password, ssl, oracleSid, insecure, caFile: ca?.path });
  const submittable = $derived(canSubmit(form));

  $effect(() => {
    void configService.config;
    void reload();
  });

  async function reload() {
    try {
      const list = await window.ocekour?.databases.list();
      if (list) databases = list;
    } catch {
      // liste inchangée
    }
  }

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

  async function test(id?: string) {
    let draft: DatabaseDraft = toDraft(form);
    if (id) {
      // Base déjà enregistrée : le cœur relit sa configuration et son mot de passe.
      const d = databases.find((x) => x.id === id)!;
      draft = { id, engine: d.engine, host: d.host, user: d.user };
    }
    const r = await run(() => window.ocekour?.databases.test(draft));
    if (r) result = r;
  }

  async function save(event?: SubmitEvent) {
    event?.preventDefault();
    if (!submittable) return;
    const id = await run(() => window.ocekour?.databases.save(toDraft(form)));
    if (!id) return;
    notifications.info(`Base « ${name.trim()} » enregistrée`);
    reset();
    await reload();
  }

  function edit(d: DatabaseInfo) {
    editingId = d.id;
    engine = d.engine;
    name = d.name;
    host = d.host;
    port = String(d.port ?? '');
    database = d.database ?? '';
    user = d.user;
    password = '';
    ssl = d.ssl;
    result = undefined;
  }

  async function remove(d: DatabaseInfo) {
    if (!confirm(`Supprimer la base « ${d.name} » ? Les agents n'y auront plus accès.`)) return;
    await run(() => window.ocekour?.databases.delete(d.id));
    if (editingId === d.id) reset();
    await reload();
  }

  function reset() {
    editingId = undefined;
    name = host = port = database = user = password = '';
    ssl = oracleSid = insecure = false;
    ca = null;
    result = undefined;
  }
</script>

<div class="max-w-3xl space-y-5 p-6 text-sm">
  <p class="text-slate-600 dark:text-slate-400">
    Donnez à vos agents un accès <strong>en lecture seule</strong> à vos bases (PostgreSQL, SQL Server, Oracle) via le serveur MCP : ils
    peuvent explorer le schéma et lancer des <code>SELECT</code>, jamais écrire. Le mot de passe est chiffré sur ce poste
    (<code>secrets.json</code>) et n'est jamais écrit dans <code>config.yaml</code>.
  </p>

  {#each databases as d (d.id)}
    <section class="rounded-lg border border-slate-200 p-4 dark:border-slate-800">
      <header class="flex flex-wrap items-center gap-2">
        <h3 class="font-semibold">{d.name}</h3>
        <span class="text-xs text-slate-500">{engineLabel(d.engine)}</span>
        {#if d.hasSecret}
          <span class="text-xs text-emerald-600">● prête</span>
        {:else}
          <span class="text-xs text-amber-600">mot de passe manquant</span>
        {/if}
        <span class="ml-auto font-mono text-xs text-slate-500">{d.user}@{d.host}:{d.port}{d.database ? '/' + d.database : ''}</span>
      </header>
      <div class="mt-2 flex flex-wrap gap-2">
        <button type="button" class="btn" disabled={busy} onclick={() => test(d.id)}>Tester</button>
        <button type="button" class="btn" onclick={() => edit(d)}>Modifier</button>
        <button type="button" class="btn" onclick={() => remove(d)}>Supprimer</button>
      </div>
    </section>
  {/each}

  <section class="rounded-lg border border-slate-200 p-4 dark:border-slate-800">
    <h3 class="mb-3 font-semibold">{editingId ? 'Modifier la base' : 'Ajouter une base'}</h3>
    <form class="grid grid-cols-[8rem_1fr] items-center gap-x-4 gap-y-3" onsubmit={save}>
      <label class="font-medium" for="db-engine">Moteur</label>
      <select id="db-engine" class="input w-56" value={engine} onchange={(e) => ((engine = e.currentTarget.value as DatabaseEngine), (result = undefined))}>
        {#each ENGINES as e (e.value)}<option value={e.value}>{e.label}</option>{/each}
      </select>

      <label class="font-medium" for="db-name">Nom</label>
      <input id="db-name" class="input w-full" placeholder="Ma base de prod" bind:value={name} />

      <label class="font-medium" for="db-host">Hôte</label>
      <input id="db-host" class="input w-full font-mono" placeholder="db.exemple.fr" bind:value={host} />

      <label class="font-medium" for="db-port">Port</label>
      <input id="db-port" type="text" inputmode="numeric" class="input w-32" placeholder={defaultPort(engine)} bind:value={port} />

      <label class="font-medium" for="db-database">{engine === 'oracle' ? (oracleSid ? 'SID' : 'Service') : 'Base'}</label>
      <div class="flex items-center gap-3">
        <input id="db-database" class="input w-full font-mono" bind:value={database} />
        {#if engine === 'oracle'}
          <label class="flex shrink-0 items-center gap-1.5 text-xs"><input type="checkbox" bind:checked={oracleSid} /> SID</label>
        {/if}
      </div>

      <label class="font-medium" for="db-user">Utilisateur</label>
      <input id="db-user" class="input w-full font-mono" autocomplete="off" bind:value={user} />

      <label class="font-medium" for="db-password">Mot de passe</label>
      <input id="db-password" type="password" class="input w-full font-mono" autocomplete="off" placeholder={editingId ? 'inchangé si vide' : ''} bind:value={password} />

      <span class="font-medium">Chiffrement</span>
      <div class="space-y-2">
        <label class="flex items-center gap-1.5"><input type="checkbox" bind:checked={ssl} /> Connexion TLS/SSL</label>
        {#if ssl}<TlsField name="db-tls" bind:insecure bind:ca />{/if}
      </div>

      <div></div>
      <div class="flex flex-wrap items-center gap-2">
        <button type="button" class="btn" disabled={busy || !submittable} onclick={() => test()}>Tester</button>
        <button type="submit" class="btn btn-primary" disabled={busy || !submittable}>{editingId ? 'Enregistrer' : 'Ajouter'}</button>
        {#if editingId}<button type="button" class="btn" onclick={reset}>Annuler</button>{/if}
        {#if result}
          {#if result.ok}
            <span class="text-emerald-600">✓ {result.summary}</span>
          {:else}
            <span class="text-red-600">{result.error}</span>
          {/if}
        {/if}
      </div>
    </form>
  </section>
</div>
