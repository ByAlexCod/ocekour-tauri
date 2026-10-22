<script lang="ts">
  import { onMount } from 'svelte';
  import type { NotificationSource, NotifierStatus } from '../../../shared/api';
  import { configService } from '../core/config.svelte';
  import { notifications } from '../core/notification.svelte';

  /**
   * Paramètres → Notifications : Ocekour surveille GitLab, Jira et Confluence et envoie, pour
   * ce qui vous concerne directement, un message groupé à vous-même dans Rocket.Chat.
   */
  const sources: { id: NotificationSource; label: string; hint: string }[] = [
    { id: 'gitlab', label: 'GitLab', hint: 'assigné, revue demandée, mention, pipeline en échec' },
    { id: 'jira', label: 'Jira', hint: 'assigné, commentaire, mention, changement de statut' },
    { id: 'confluence', label: 'Confluence', hint: 'mention, commentaire, page suivie modifiée' },
  ];
  let status = $state.raw<NotifierStatus | undefined>(undefined);
  let testing = $state(false);
  const actives = $derived(new Set(status?.actives ?? []));
  const n = $derived(configService.config?.notifications);

  // État relu à chaque changement de configuration, puis suivi en direct.
  $effect(() => {
    void configService.config;
    void window.ocekour?.notifications
      .status()
      .then((s) => (status = s))
      .catch(() => undefined);
  });
  onMount(() => window.ocekour?.notifications.onChanged((s) => (status = s)));

  const patch = (p: Record<string, unknown>) => void window.ocekour?.config.setNotifications(p).catch((e) => notifications.error(e));

  function setInterval(input: HTMLInputElement) {
    const value = input.valueAsNumber;
    if (Number.isInteger(value) && value >= 30 && value <= 3_600) patch({ intervalSeconds: value });
  }

  async function test() {
    testing = true;
    try {
      const result = await window.ocekour?.notifications.test();
      if (result?.ok) notifications.info('Message d’essai envoyé dans Rocket.Chat');
      else if (result) notifications.error(result.error ?? 'Échec');
    } catch (e) {
      notifications.error(e);
    } finally {
      testing = false;
    }
  }

  const time = (iso: string) => new Date(iso).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
</script>

{#if n}
  <div class="space-y-3">
    <label class="flex items-center gap-1.5">
      <input type="checkbox" checked={n.enabled} onchange={(e) => patch({ enabled: e.currentTarget.checked })} />
      Surveiller GitLab, Jira et Confluence et m'envoyer les nouveautés dans Rocket.Chat
    </label>

    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-slate-500">
      <label class="flex items-center gap-1.5">
        Toutes les
        <input type="number" class="input w-20 py-0.5" min="30" max="3600" step="30" value={n.intervalSeconds} onchange={(e) => setInterval(e.currentTarget)} />
        secondes
      </label>
      <label class="flex items-center gap-1.5">
        Destinataire
        <input type="text" class="input w-40 py-0.5 font-mono" value={n.target} onchange={(e) => patch({ target: e.currentTarget.value.trim() || 'self' })} />
      </label>
      <span title="self = à moi-même ; sinon #canal ou @personne"><code>self</code> = à moi-même</span>
    </div>

    <label class="flex items-center gap-1.5">
      <input type="checkbox" checked={n.desktop} onchange={(e) => patch({ desktop: e.currentTarget.checked })} />
      Afficher aussi les nouveautés en notifications Windows
    </label>

    <div class="space-y-1">
      {#each sources as row (row.id)}
        <label class="flex items-center gap-1.5 text-sm">
          <input type="checkbox" checked={n.sources[row.id].enabled} onchange={(e) => patch({ sources: { [row.id]: { enabled: e.currentTarget.checked } } })} />
          {row.label}
          <span class="text-xs text-slate-500">— {row.hint}</span>
          {#if n.enabled && n.sources[row.id].enabled && !actives.has(row.id)}<span class="text-xs text-amber-600">(jeton manquant — Comptes)</span>{/if}
        </label>
      {/each}
    </div>

    <div class="flex flex-wrap items-center gap-2 text-xs">
      <button type="button" class="btn py-0.5" disabled={testing} onclick={test}>Envoyer un message d'essai</button>
      {#if status?.lastError}
        <span class="text-red-600">{status.lastError}</span>
      {:else if status?.running}
        <span class="text-emerald-600">● actif</span>
        {#if status.lastRun}
          <span class="text-slate-500">· dernier passage {time(status.lastRun)}{#if status.lastCount}, {status.lastCount} envoyée(s){/if}</span>
        {/if}
      {:else if n.enabled}
        <span class="text-slate-500">démarrage…</span>
      {:else}
        <span class="text-slate-500">arrêté</span>
      {/if}
    </div>

    <p class="text-xs text-slate-500">
      Seulement ce qui vous concerne directement (assigné, mention, revue demandée…). Un seul message groupé par passage. Les notifications ne partent que pendant qu'Ocekour tourne
      (fenêtre ouverte, ou <code>--notify</code> sans interface). Rien n'est envoyé aux agents.
    </p>
  </div>
{/if}
