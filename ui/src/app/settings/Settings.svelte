<script lang="ts" module>
  export type SettingsTab = 'preferences' | 'accounts' | 'databases' | 'yaml';
</script>

<script lang="ts">
  import type { Preferences, Theme } from '../../../shared/api';
  import { configService } from '../core/config.svelte';
  import { licence } from '../core/licence.svelte';
  import { themeService } from '../core/theme.svelte';
  import { MODE_LABELS } from '../licence/Activation.svelte';
  import Accounts from './Accounts.svelte';
  import AgentFilesSettings from './AgentFilesSettings.svelte';
  import ContextSettings from './ContextSettings.svelte';
  import DatabaseSettings from './DatabaseSettings.svelte';
  import McpSettings from './McpSettings.svelte';
  import NotificationSettings from './NotificationSettings.svelte';
  import YamlEditor from './YamlEditor.svelte';

  let { tab = $bindable('preferences'), onshowactions }: { tab?: SettingsTab; onshowactions: () => void } = $props();

  const tabs: { id: SettingsTab; label: string }[] = [
    { id: 'preferences', label: 'Préférences' },
    { id: 'accounts', label: 'Comptes' },
    { id: 'databases', label: 'Bases de données' },
    { id: 'yaml', label: 'config.yaml' },
  ];
  const themes: { value: Theme; label: string }[] = [
    { value: 'system', label: 'Système' },
    { value: 'light', label: 'Clair' },
    { value: 'dark', label: 'Sombre' },
  ];

  function setNumber(key: 'bufferSize' | 'defaultTailLines', input: HTMLInputElement) {
    const value = input.valueAsNumber;
    if (Number.isInteger(value)) set({ [key]: value });
  }

  const set = (patch: Partial<Preferences>) => configService.setPreferences(patch);
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <nav class="flex gap-4 border-b border-slate-200 px-4 text-sm dark:border-slate-800">
    {#each tabs as t (t.id)}
      <button
        type="button"
        class="-mb-px border-b-2 border-transparent py-2 text-slate-500 hover:text-slate-900 aria-selected:border-sky-500 aria-selected:text-slate-900 dark:hover:text-slate-100 dark:aria-selected:text-slate-100"
        role="tab"
        aria-selected={tab === t.id}
        onclick={() => (tab = t.id)}>{t.label}</button
      >
    {/each}
  </nav>

  {#if tab === 'yaml'}
    <YamlEditor />
  {:else if tab === 'accounts'}
    <div class="min-h-0 flex-1 overflow-y-auto"><Accounts {onshowactions} /></div>
  {:else if tab === 'databases'}
    <div class="min-h-0 flex-1 overflow-y-auto"><DatabaseSettings /></div>
  {:else if configService.config}
    {@const config = configService.config}
    <div class="min-h-0 flex-1 overflow-y-auto">
      <form class="grid max-w-2xl grid-cols-[14rem_1fr] items-center gap-x-6 gap-y-5 p-6 text-sm" onsubmit={(e) => e.preventDefault()}>
        <span class="font-medium">Thème</span>
        <div class="flex gap-4">
          {#each themes as t (t.value)}
            <label class="flex items-center gap-1.5">
              <input type="radio" name="theme" checked={config.theme === t.value} onchange={() => themeService.set(t.value)} />
              {t.label}
            </label>
          {/each}
        </div>

        <label for="tail" class="font-medium">Lignes chargées à l'ouverture</label>
        <input id="tail" type="number" min="0" step="100" class="input w-32" value={config.defaultTailLines} onchange={(e) => setNumber('defaultTailLines', e.currentTarget)} />

        <label for="buffer" class="font-medium">Lignes gardées par onglet</label>
        <div>
          <input id="buffer" type="number" min="1000" step="10000" class="input w-32" value={config.bufferSize} onchange={(e) => setNumber('bufferSize', e.currentTarget)} />
          <p class="mt-1 text-xs text-slate-500">Au-delà, les plus anciennes sont retirées de la mémoire.</p>
        </div>

        <span class="font-medium">Session</span>
        <label class="flex items-center gap-1.5">
          <input type="checkbox" checked={config.restoreTabs} onchange={(e) => set({ restoreTabs: e.currentTarget.checked })} />
          Rouvrir les onglets au démarrage
        </label>

        <label for="share" class="font-medium">Dossier des mises à jour</label>
        <div>
          <input id="share" type="text" class="input w-full font-mono" placeholder={'\\\\serveur\\partage\\ocekour'} value={config.updateShare ?? ''} onchange={(e) => set({ updateShare: e.currentTarget.value.trim() })} />
          <p class="mt-1 text-xs text-slate-500">Optionnel. Réservé à une prochaine version (détection d'un nouvel exe au démarrage).</p>
        </div>

        <span class="font-medium">Licence</span>
        <div>
          {#if licence.state.info}
            {@const info = licence.state.info}
            <p>Clé « {info.label} » · <strong>{MODE_LABELS[info.mode]}</strong></p>
            <p class="mt-1 text-xs text-slate-500">
              {info.id} · émise le {info.issuedAt}.
              {#if info.mode === 'none'}
                Aucune statistique n'est envoyée.
              {:else}
                Statistiques d'utilisation anonymes envoyées au serveur de l'équipe {info.mode === 'required' ? '(obligatoire pour utiliser Ocekour)' : '(si joignable)'} : écrans et fonctions
                utilisés, codes d'erreur, version. Jamais de secret, de nom de serveur, de topic, de bucket, de texte recherché ni de contenu de log.
              {/if}
            </p>
          {/if}
          <button type="button" class="btn mt-2" onclick={() => (licence.changing = true)}>Changer de clé</button>
        </div>

        <span class="font-medium">Notifications</span>
        <NotificationSettings />

        <span class="font-medium">Contexte IA</span>
        <ContextSettings />

        <span class="font-medium">Serveur MCP</span>
        <McpSettings />

        <span class="font-medium">Fichiers pour les agents</span>
        <AgentFilesSettings />

        <span class="font-medium">Journal</span>
        <div>
          <button type="button" class="btn" onclick={() => configService.showLogFile()}>Afficher le journal</button>
          <p class="mt-1 text-xs text-slate-500"><code>logs\main.log</code> : à joindre pour signaler un problème (aucun secret dedans).</p>
        </div>
      </form>
    </div>
  {:else}
    <p class="p-6 text-sm text-slate-500">Configuration disponible uniquement dans l'application.</p>
  {/if}
</div>
