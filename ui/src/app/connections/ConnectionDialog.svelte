<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type { CaFileInfo, Connection, ConnectionDraft, ConnectionType, KafkaDraft, RancherCluster, SourceErrorInfo, TlsOptions } from '../../../shared/api';
  import { errorMessage, notifications } from '../core/notification.svelte';
  import type { ConnectionPrefill } from './connection-dialog.service';
  import { CONNECTION_TYPES, SECRET_LABELS, defaultName, splitList } from './connections';
  import TlsField from './TlsField.svelte';

  type Status = { kind: 'idle' } | { kind: 'testing' } | { kind: 'ok'; summary: string } | { kind: 'error'; error: SourceErrorInfo };

  /** Création / modification d'une connexion Rancher, Kafka ou S3. */
  let {
    connection = null,
    focusToken = false,
    prefill,
    onclose,
  }: {
    /** Connexion à modifier ; `null` pour en créer une. */
    connection?: Connection | null;
    /** Ouvre directement sur le champ secret (token expiré, mot de passe changé). */
    focusToken?: boolean;
    /** Valeurs proposées (lien de partage) : type, URL ou brokers, cluster à cocher. */
    prefill?: ConnectionPrefill;
    onclose: () => void;
  } = $props();

  const api = window.ocekour!.connections;
  let dialog: HTMLDialogElement;
  let secretInput = $state<HTMLInputElement>();

  let type = $state<ConnectionType>('rancher');
  let name = $state('');
  let secret = $state('');
  let insecure = $state(false);
  let ca = $state<CaFileInfo | null>(null);
  let url = $state('');
  let selected = $state.raw<Map<string, string>>(new Map());
  let clusters = $state.raw<RancherCluster[] | null>(null);
  let clusterFilter = $state('');
  let brokers = $state('');
  let protocol = $state<KafkaDraft['security']['protocol']>('SASL_PLAINTEXT');
  let mechanism = $state<KafkaDraft['security']['mechanism']>('SCRAM-SHA-512');
  let username = $state('');
  let registryUrl = $state('');
  let registryInsecure = $state(false);
  let registryCa = $state<CaFileInfo | null>(null);
  let region = $state('');
  let pathStyle = $state(true);
  let accessKeyId = $state('');
  let buckets = $state('');
  let status = $state<Status>({ kind: 'idle' });
  let saving = $state(false);

  const isNew = $derived(!connection);
  const error = $derived(status.kind === 'error' ? status.error : null);
  const summary = $derived(status.kind === 'ok' ? status.summary : null);
  const visibleClusters = $derived.by(() => {
    const f = clusterFilter.toLowerCase();
    return (clusters ?? []).filter((c) => c.name.toLowerCase().includes(f) || c.id.toLowerCase().includes(f));
  });
  const canTest = $derived(type === 'kafka' ? !!brokers.trim() : !!url.trim());
  const canSave = $derived(!saving && !!name.trim() && canTest && (type !== 'rancher' || selected.size > 0) && (type !== 's3' || !!accessKeyId.trim()));

  onMount(async () => {
    dialog.showModal();
    if (connection) await fill(connection);
    if (prefill) applyPrefill(prefill);
    if (!connection) return;
    await tick();
    if (focusToken) secretInput?.focus();
    else if (connection.type === 'rancher') await test();
  });

  /** Change de type (nouvelle connexion) : l'état du test ne vaut plus. */
  function setType(t: ConnectionType) {
    type = t;
    status = { kind: 'idle' };
  }

  function applyPrefill(p: ConnectionPrefill) {
    if (!connection) type = p.type;
    if (p.url) url = p.url;
    if (p.brokers) brokers = p.brokers.join(', ');
    if (p.cluster) selected = new Map(selected).set(p.cluster.id, p.cluster.name);
  }

  async function fill(c: Connection) {
    type = c.type;
    name = c.name;
    insecure = c.tls.insecure;
    ca = await readCa(c.tls);
    switch (c.type) {
      case 'rancher':
        url = c.url;
        selected = new Map(c.clusters.map((k) => [k.id, k.name]));
        break;
      case 'kafka':
        brokers = c.brokers.join(', ');
        protocol = c.security.protocol;
        mechanism = c.security.mechanism;
        username = c.security.username ?? '';
        registryUrl = c.registry?.url ?? '';
        registryInsecure = c.registry?.tls.insecure ?? false;
        registryCa = c.registry ? await readCa(c.registry.tls) : null;
        break;
      case 's3':
        url = c.url;
        region = c.region ?? '';
        pathStyle = c.pathStyle;
        accessKeyId = c.accessKeyId;
        buckets = c.buckets.join('\n');
        break;
    }
  }

  async function readCa(tls: TlsOptions): Promise<CaFileInfo | null> {
    if (!tls.caFile) return null;
    try {
      return await api.readCaFile(tls.caFile);
    } catch (e) {
      notifications.error(e);
      return null;
    }
  }

  function draft(): ConnectionDraft {
    const tls = (ins: boolean, info: CaFileInfo | null): TlsOptions => ({ insecure: ins, caFile: ins ? undefined : info?.path });
    const base = { id: connection?.id, name: name.trim(), secret: secret.trim() || undefined, tls: tls(insecure, ca) };
    switch (type) {
      case 'rancher':
        return { ...base, type: 'rancher', url: url.trim(), clusters: [...selected].map(([id, n]) => ({ id, name: n })) };
      case 'kafka':
        return {
          ...base,
          type: 'kafka',
          brokers: splitList(brokers),
          security: { protocol, mechanism, username: username.trim() || undefined },
          registry: registryUrl.trim() ? { url: registryUrl.trim(), tls: tls(registryInsecure, registryCa) } : undefined,
        };
      case 's3':
        return {
          ...base,
          type: 's3',
          url: url.trim(),
          region: region.trim() || undefined,
          pathStyle,
          accessKeyId: accessKeyId.trim(),
          buckets: splitList(buckets),
        };
    }
  }

  /** Teste la connexion ; pour Rancher, charge ensuite la liste des clusters. */
  async function test() {
    status = { kind: 'testing' };
    const d = draft();
    try {
      const result = await api.test(d);
      if (!result.ok) {
        status = { kind: 'error', error: result.error };
        if (result.error.code === 'unauthorized') secretInput?.focus();
        return;
      }
      if (d.type === 'rancher') {
        const list = await api.listClusters(d);
        if (!list.ok) {
          status = { kind: 'error', error: list.error };
          return;
        }
        clusters = list.value;
      }
      status = { kind: 'ok', summary: result.value.summary };
      if (!name) name = defaultName(d);
    } catch (e) {
      status = { kind: 'error', error: { code: 'invalid', message: errorMessage(e) } };
    }
  }

  function toggle(cluster: RancherCluster, checked: boolean) {
    const next = new Map(selected);
    if (checked) next.set(cluster.id, cluster.name);
    else next.delete(cluster.id);
    selected = next;
  }

  function selectVisible(checked: boolean) {
    for (const c of visibleClusters) toggle(c, checked);
  }

  async function importKubeconfig() {
    try {
      const d = await api.importKubeconfig();
      if (!d) return;
      type = 'rancher';
      name = d.name;
      url = d.url;
      secret = d.secret ?? '';
      selected = new Map(d.clusters.map((c) => [c.id, c.name]));
      await test();
    } catch (e) {
      notifications.error(e);
    }
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    saving = true;
    try {
      await api.save(draft());
      notifications.info(`Connexion « ${name} » enregistrée`);
      close();
    } catch (e) {
      notifications.error(errorMessage(e));
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!connection) return;
    const what = SECRET_LABELS[connection.type].toLowerCase();
    if (!confirm(`Supprimer la connexion « ${connection.name} » et son ${what} ?`)) return;
    try {
      await api.delete(connection.id);
      close();
    } catch (e) {
      notifications.error(e);
    }
  }

  /** Échap ferme aussi la boîte : `onclose` suit l'événement natif `close`. */
  function close() {
    dialog.close();
  }
</script>

<dialog
  bind:this={dialog}
  onclose={onclose}
  class="m-auto w-[42rem] max-w-[95vw] rounded-lg border border-slate-200 bg-white p-0 text-slate-900 shadow-2xl backdrop:bg-black/50 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-100"
>
  <form class="flex max-h-[90vh] flex-col" onsubmit={save}>
    <header class="flex items-center justify-between border-b border-slate-200 px-5 py-3 dark:border-slate-800">
      <h2 class="font-semibold">{isNew ? 'Nouvelle connexion' : 'Modifier « ' + connection?.name + ' »'}</h2>
      {#if isNew && type === 'rancher'}
        <button type="button" class="btn" onclick={importKubeconfig}>Importer un kubeconfig…</button>
      {/if}
    </header>

    <div class="grid grid-cols-[10rem_1fr] items-start gap-x-4 gap-y-4 overflow-y-auto px-5 py-4 text-sm">
      {#if isNew}
        <span class="pt-1.5 font-medium">Type</span>
        <div class="grid grid-cols-3 gap-2" role="radiogroup" aria-label="Type de connexion">
          {#each CONNECTION_TYPES as t (t.type)}
            <button
              type="button"
              role="radio"
              class="rounded-md border px-3 py-2 text-left hover:bg-slate-50 aria-checked:border-sky-500 aria-checked:bg-sky-50 dark:hover:bg-slate-800 dark:aria-checked:bg-sky-950 {type === t.type
                ? ''
                : 'border-slate-200 dark:border-slate-700'}"
              aria-checked={type === t.type}
              onclick={() => setType(t.type)}
            >
              <span class="block font-medium">{t.label}</span>
              <span class="block text-xs text-slate-500">{t.hint}</span>
            </button>
          {/each}
        </div>
      {/if}

      {#if type === 'kafka'}
        <label for="c-brokers" class="pt-1.5 font-medium">Brokers</label>
        <div>
          <input id="c-brokers" class="input w-full font-mono" placeholder="kafka1.intra:9092, kafka2.intra:9092" bind:value={brokers} />
          <p class="mt-1 text-xs text-slate-500">bootstrap.servers, séparés par des virgules.</p>
        </div>
        <label for="c-protocol" class="pt-1.5 font-medium">Sécurité</label>
        <div class="flex flex-wrap gap-2">
          <select id="c-protocol" class="input" aria-label="security.protocol" bind:value={protocol}>
            {#each ['SASL_PLAINTEXT', 'SASL_SSL', 'PLAINTEXT'] as p (p)}<option value={p}>{p}</option>{/each}
          </select>
          {#if protocol !== 'PLAINTEXT'}
            <select class="input" aria-label="sasl.mechanism" bind:value={mechanism}>
              {#each ['SCRAM-SHA-512', 'SCRAM-SHA-256', 'PLAIN'] as m (m)}<option value={m}>{m}</option>{/each}
            </select>
          {/if}
        </div>
        {#if protocol !== 'PLAINTEXT'}
          <label for="c-user" class="pt-1.5 font-medium">Utilisateur</label>
          <input id="c-user" class="input" autocomplete="off" bind:value={username} />
        {/if}
      {:else}
        <label for="c-url" class="pt-1.5 font-medium">{type === 'rancher' ? 'URL Rancher' : 'URL du S3'}</label>
        <input id="c-url" class="input" type="url" placeholder={type === 'rancher' ? 'https://rancher.mon-entreprise.fr' : 'https://s3.mon-entreprise.fr'} bind:value={url} />
        {#if type === 's3'}
          <label for="c-access" class="pt-1.5 font-medium">Clé d'accès</label>
          <input id="c-access" class="input font-mono" autocomplete="off" bind:value={accessKeyId} />
        {/if}
      {/if}

      {#if type !== 'kafka' || protocol !== 'PLAINTEXT'}
        <label for="c-secret" class="pt-1.5 font-medium">{SECRET_LABELS[type]}</label>
        <div>
          <input
            bind:this={secretInput}
            id="c-secret"
            class="input w-full font-mono"
            class:border-red-500!={error?.code === 'unauthorized'}
            type="password"
            autocomplete="off"
            placeholder={isNew ? '' : 'Enregistré — laisser vide pour le conserver'}
            bind:value={secret}
          />
          <p class="mt-1 text-xs text-slate-500">
            {#if type === 'rancher'}Rancher › avatar › <em>API &amp; Keys</em> › <em>Add Key</em>, sans restriction de cluster.{/if}
            Stocké chiffré sur ce poste, jamais dans config.yaml.
          </p>
        </div>
      {/if}

      {#if type !== 'kafka' || protocol === 'SASL_SSL'}
        <span class="pt-1 font-medium">Certificat SSL</span>
        <TlsField name="tls" bind:insecure bind:ca />
      {/if}

      {#if type === 'kafka'}
        <label for="c-registry" class="pt-1.5 font-medium">Schema Registry</label>
        <div>
          <input id="c-registry" class="input w-full" type="url" placeholder="http://schema-registry.intra:8081 (facultatif, pour Avro)" bind:value={registryUrl} />
          <p class="mt-1 text-xs text-slate-500">Sans authentification.</p>
        </div>
        {#if registryUrl.trim().startsWith('https')}
          <span class="pt-1 font-medium">Certificat du registre</span>
          <TlsField name="registry-tls" bind:insecure={registryInsecure} bind:ca={registryCa} />
        {/if}
      {/if}

      {#if type === 's3'}
        <label for="c-region" class="pt-1.5 font-medium">Région</label>
        <input id="c-region" class="input w-40" placeholder="facultative" bind:value={region} />
        <span class="pt-0.5 font-medium">Adressage</span>
        <label class="flex items-center gap-2">
          <input type="checkbox" bind:checked={pathStyle} />
          Path-style (<code class="text-xs">https://s3.intra/bucket/clé</code>)
        </label>
      {/if}

      <span></span>
      <div class="flex items-center gap-3">
        <button type="button" class="btn" disabled={!canTest || status.kind === 'testing'} onclick={test}>
          {type === 'rancher' ? 'Tester et charger les clusters' : 'Tester la connexion'}
        </button>
        {#if status.kind === 'testing'}
          <span class="text-slate-500">Connexion…</span>
        {:else if summary}
          <span class="text-emerald-600 dark:text-emerald-400">✓ {summary}</span>
        {/if}
      </div>
      {#if error}
        <span></span>
        <p role="alert" class="rounded bg-red-50 px-3 py-2 text-red-700 dark:bg-red-950 dark:text-red-300">{error.message}</p>
      {/if}

      <label for="c-name" class="pt-1.5 font-medium">Nom</label>
      <input id="c-name" class="input" required placeholder={type === 'rancher' ? 'Rancher prod' : type === 'kafka' ? 'Kafka prod' : 'S3 prod'} bind:value={name} />

      {#if type === 'rancher'}
        <span class="pt-1 font-medium">Clusters <span class="font-normal text-slate-500">({selected.size})</span></span>
        {#if clusters}
          <div class="rounded-md border border-slate-200 dark:border-slate-700">
            <div class="flex items-center gap-2 border-b border-slate-200 p-2 dark:border-slate-700">
              <input class="input flex-1" type="search" placeholder="Filtrer…" bind:value={clusterFilter} />
              <button type="button" class="btn" onclick={() => selectVisible(true)}>Tout cocher</button>
              <button type="button" class="btn" onclick={() => selectVisible(false)}>Aucun</button>
            </div>
            <ul class="max-h-56 overflow-y-auto p-1">
              {#each visibleClusters as c (c.id)}
                <li>
                  <label class="flex items-center gap-2 rounded px-2 py-1 hover:bg-slate-100 dark:hover:bg-slate-800">
                    <input type="checkbox" checked={selected.has(c.id)} onchange={(e) => toggle(c, e.currentTarget.checked)} />
                    <span class="flex-1">{c.name} <span class="text-xs text-slate-500">{c.id}</span></span>
                    {#if c.state !== 'active'}<span class="text-xs text-amber-600">{c.state}</span>{/if}
                  </label>
                </li>
              {:else}
                <li class="px-2 py-1 text-slate-500">Aucun cluster.</li>
              {/each}
            </ul>
          </div>
        {:else}
          <p class="pt-1 text-slate-500">Testez la connexion pour choisir les clusters.</p>
        {/if}
      {/if}

      {#if type === 's3'}
        <label for="c-buckets" class="pt-1.5 font-medium">Buckets en plus</label>
        <div>
          <textarea id="c-buckets" class="input w-full font-mono" rows="2" placeholder="un par ligne (facultatif)" bind:value={buckets}></textarea>
          <p class="mt-1 text-xs text-slate-500">Buckets à afficher même si le compte ne peut pas lister les buckets.</p>
        </div>
      {/if}
    </div>

    <footer class="flex items-center gap-2 border-t border-slate-200 px-5 py-3 dark:border-slate-800">
      {#if !isNew}<button type="button" class="btn text-red-600" onclick={remove}>Supprimer</button>{/if}
      <span class="flex-1"></span>
      <button type="button" class="btn" onclick={close}>Annuler</button>
      <button type="submit" class="btn-primary" disabled={!canSave}>Enregistrer</button>
    </footer>
  </form>
</dialog>
