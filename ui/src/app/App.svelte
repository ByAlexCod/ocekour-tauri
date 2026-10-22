<script lang="ts">
  import type { Theme } from '../../shared/api';
  import { actionsService as actions } from './actions/actions.service';
  import { analytics } from './core/analytics';
  import { configService } from './core/config.svelte';
  import { licence } from './core/licence.svelte';
  import { themeService } from './core/theme.svelte';
  import { KonamiDetector } from './fun/snake';
  import Activation from './licence/Activation.svelte';
  import { AssemblySession } from './logs/assembly-session';
  import { LogSession } from './logs/log-session';
  import LogView from './logs/LogView.svelte';
  import TabBar from './logs/TabBar.svelte';
  import { tabsService as tabs } from './logs/tabs.service';
  import type { SettingsTab } from './settings/Settings.svelte';
  import { localFiles } from './files/local-files.svelte';
  import { findLink } from './share/share-link';
  import { shareService } from './share/share.service';
  import Resizer from './shared/Resizer.svelte';
  import { storedWidth } from './shared/resizer';
  import Toasts from './shared/Toasts.svelte';
  import SourcesPanel from './sources/SourcesPanel.svelte';

  const themes: { value: Theme; label: string }[] = [
    { value: 'system', label: 'Système' },
    { value: 'light', label: 'Clair' },
    { value: 'dark', label: 'Sombre' },
  ];

  let view = $state<'logs' | 'settings' | 'actions'>('logs');
  /** Largeur de la liste des connexions (mémorisée sur le poste). */
  let sidebarWidth = $state(storedWidth('ocekour.sidebar.width', 320));
  let settingsTab = $state<SettingsTab>('preferences');
  /** ↑ ↑ ↓ ↓ ← → ← → B A. */
  let snake = $state(false);
  const konami = new KonamiDetector();
  // Écrans secondaires chargés à la demande (éditeur CodeMirror, explorateur) : démarrage plus rapide.
  const lazy = {
    settings: () => import('./settings/Settings.svelte'),
    actions: () => import('./actions/ActionsView.svelte'),
    explorer: () => import('./explorer/ExplorerView.svelte'),
    snake: () => import('./fun/Snake.svelte'),
  };

  const active = $derived(tabs.active());
  const configError = $derived(configService.errors[0]);
  const unseen = $derived(actions.unseen());

  // Statistiques : écran affiché (type d'onglet, jamais son contenu).
  $effect(() => {
    if (view === 'actions') return analytics.page("Actions de l'agent", 'actions');
    if (view === 'settings') return analytics.page('Paramètres', `parametres/${settingsTab}`);
    const target = active?.target;
    if (!target) analytics.page('Accueil', 'accueil');
    else if (target.kind === 'container') analytics.page('Logs Rancher', 'logs/rancher');
    else if (target.kind === 'topic') analytics.page('Messages Kafka', 'logs/kafka');
    else if (target.kind === 'file') analytics.page(target.connectionId ? 'Logs de sonde' : 'Fichier local', target.connectionId ? 'logs/sonde' : 'logs/fichier');
    else analytics.page('Explorateur S3', 's3');
  });

  /** Pod supprimé : même conteneur sur un pod de remplacement du même workload. */
  function switchPod(pod: string) {
    const tab = tabs.active();
    if (!(tab instanceof LogSession) || tab.target.kind !== 'container') return;
    const target = tab.target;
    tabs.replaceTarget(tab, { ...target, pod, node: { ...target.node, hasPrevious: false } });
  }

  const isStream = (tab: unknown): tab is LogSession | AssemblySession => tab instanceof LogSession || tab instanceof AssemblySession;
  const typing = (target: EventTarget | null) => (target as HTMLElement | null)?.closest?.('input, textarea, [contenteditable="true"], .cm-editor');

  /** Ctrl+W : fermer l'onglet ; Ctrl+Tab ou Ctrl+PageSuiv / Ctrl+Maj+Tab ou Ctrl+PagePréc : suivant / précédent. */
  function onKey(event: KeyboardEvent) {
    if (!typing(event.target) && !snake && konami.push(event.key)) {
      snake = true;
      analytics.event({ category: 'Application', action: 'Code Konami' });
      return;
    }
    if (!(event.ctrlKey || event.metaKey) || view !== 'logs') return;
    if (event.key.toLowerCase() === 'o' && !event.shiftKey && !event.altKey) {
      event.preventDefault();
      void localFiles.pick(event.shiftKey);
    } else if (event.key.toLowerCase() === 'w') {
      event.preventDefault();
      tabs.close();
    } else if (event.key === 'Tab' || event.key === 'PageDown' || event.key === 'PageUp') {
      event.preventDefault();
      tabs.cycle(event.shiftKey || event.key === 'PageUp' ? -1 : 1);
    }
  }

  /** Ctrl+V hors d'un champ de saisie : ouvre le lien Ocekour collé. */
  function onPaste(event: ClipboardEvent) {
    if (typing(event.target)) return;
    const text = event.clipboardData?.getData('text/plain') ?? '';
    if (!findLink(text)) return;
    event.preventDefault();
    view = 'logs';
    shareService.open(text);
  }

  function fixConfig() {
    settingsTab = 'yaml';
    view = 'settings';
  }
</script>

<svelte:window onkeydown={onKey} />
<svelte:document onpaste={onPaste} />

<div class="flex h-full flex-col">
  <header class="flex h-12 shrink-0 items-center justify-between border-b border-slate-200 px-4 dark:border-slate-800">
    <div class="flex items-center gap-6">
      <div class="flex items-center gap-2">
        <img src="icon.png" alt="" class="size-6" />
        <h1 class="font-semibold tracking-tight">Ocekour</h1>
      </div>
      <nav class="flex gap-1 text-sm">
        <button type="button" class="nav-link" aria-current={view === 'logs' ? 'page' : null} onclick={() => (view = 'logs')}>Logs</button>
        <button type="button" class="nav-link" aria-current={view === 'settings' ? 'page' : null} onclick={() => (view = 'settings')}>Paramètres</button>
        <button type="button" class="nav-link flex items-center gap-1.5" aria-current={view === 'actions' ? 'page' : null} onclick={() => (view = 'actions')}>
          Actions de l'agent
          {#if unseen && view !== 'actions'}
            <span class="rounded-full bg-sky-600 px-1.5 text-xs leading-4 text-white" title="{unseen} nouvelle(s) action(s)">{unseen}</span>
          {/if}
        </button>
      </nav>
    </div>

    <div class="flex rounded-md border border-slate-200 p-0.5 text-sm dark:border-slate-800" role="radiogroup" aria-label="Thème">
      {#each themes as t (t.value)}
        <button
          type="button"
          role="radio"
          aria-checked={themeService.theme === t.value}
          onclick={() => themeService.set(t.value)}
          class="rounded px-2.5 py-1 text-slate-600 hover:text-slate-900 aria-checked:bg-slate-100 aria-checked:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100 dark:aria-checked:bg-slate-800 dark:aria-checked:text-slate-100"
        >
          {t.label}
        </button>
      {/each}
    </div>
  </header>

  {#if configError}
    <div
      role="alert"
      class="flex items-center gap-3 border-b border-amber-300 bg-amber-50 px-4 py-2 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200"
    >
      <span class="flex-1">
        <strong>config.yaml contient une erreur{configError.line ? ` (ligne ${configError.line})` : ''}</strong>
        : {configError.message}. La dernière configuration valide reste utilisée.
      </span>
      <button type="button" class="btn" onclick={fixConfig}>Corriger</button>
    </div>
  {/if}

  {#if view === 'settings'}
    {#await lazy.settings() then { default: Settings }}
      <Settings bind:tab={settingsTab} onshowactions={() => (view = 'actions')} />
    {/await}
  {:else if view === 'actions'}
    {#await lazy.actions() then { default: ActionsView }}<ActionsView />{/await}
  {:else}
    <div class="flex min-h-0 flex-1">
      <aside class="flex shrink-0 flex-col overflow-hidden bg-slate-50 dark:bg-slate-900" style:width="{sidebarWidth}px">
        <SourcesPanel />
      </aside>
      <Resizer label="Redimensionner la liste des connexions" storageKey="ocekour.sidebar.width" defaultWidth={320} bind:width={sidebarWidth} />

      <main class="flex min-w-0 flex-1 flex-col">
        {#if tabs.tabs().length}<TabBar />{/if}
        {#if active}
          <!-- Vue recréée à chaque changement d'onglet. -->
          {#key active}
            {#if isStream(active)}
              <LogView session={active} onswitchpod={switchPod} oneditconfig={fixConfig} />
            {:else}
              {#await lazy.explorer() then { default: ExplorerView }}<ExplorerView session={active} />{/await}
            {/if}
          {/key}
        {:else}
          <div class="m-auto max-w-sm text-center text-sm text-slate-500">
            <p>Sélectionnez un conteneur, un topic ou un bucket.</p>
            <p class="mt-3"><button type="button" class="btn" onclick={() => tabs.startAssembly()}>⊕ Nouvel assemblage</button></p>
            <p class="mt-1 text-xs">Un assemblage fusionne plusieurs pods / topics dans une vue triée dans le temps.</p>
            <p class="mt-3"><button type="button" class="btn" onclick={() => localFiles.pick()}>📄 Ouvrir un fichier de logs…</button></p>
            <p class="mt-1 text-xs">Ou glissez des fichiers sur la fenêtre (Ctrl+O).</p>
            <p class="mt-2 text-xs">Ctrl+clic ou clic milieu : nouvel onglet · Ctrl+W : fermer · Ctrl+Tab : onglet suivant</p>
          </div>
        {/if}
      </main>
    </div>
  {/if}
</div>

{#if licence.state.status !== 'active' || licence.changing}
  <Activation />
{/if}
{#if snake}
  {#await lazy.snake() then { default: Snake }}<Snake onclose={() => (snake = false)} />{/await}
{/if}
<Toasts />
