import { flushSync, mount, tick, unmount } from 'svelte';
import type { ConfigState, OcekourApi, Preferences } from '../../shared/api';
import App from './App.svelte';
import { startServices } from './bootstrap';

// Faux process main : config en mémoire.
export function fakeApi(): OcekourApi {
  let state: ConfigState = {
    config: {
      theme: 'system',
      bufferSize: 100_000,
      defaultTailLines: 1_000,
      restoreTabs: true,
      mcpServer: false,
      mcpPort: 47110,
      connections: [],
      profiles: [],
      favorites: [],
      filterPresets: [],
      web: { enabled: false, browser: 'msedge', headless: false, timeoutMs: 30_000, tls: { insecure: false } },
      databases: [],
      services: {},
      notifications: {
        enabled: false,
        intervalSeconds: 60,
        target: 'self',
        desktop: true,
        sources: {
          gitlab: { enabled: true, events: [] },
          jira: { enabled: true, events: [] },
          confluence: { enabled: true, events: [] },
        },
      },
      contexte: {
        enabled: false,
        intervalSeconds: 60,
        llm: 'auto',
        tls: { insecure: false },
        sources: {
          rocketchat: { enabled: true },
          gitlab: { enabled: true },
          jira: { enabled: true },
          confluence: { enabled: true },
        },
      },
    },
    errors: [],
    path: 'config.yaml',
  };
  return {
    config: {
      get: async () => state,
      setPreferences: async (patch: Partial<Preferences>) =>
        (state = { ...state, config: { ...state.config, ...patch } }),
      setNotifications: async () => state,
      setContexte: async () => state,
      readText: async () => '',
      validateText: async () => [],
      saveText: async () => ({ ok: true, state }),
      toggleFavorite: async () => state,
      editProfile: async () => state,
      saveFilterPreset: async () => state,
      deleteFilterPreset: async () => state,
      renameFilterPreset: async () => state,
      onChanged: () => () => {},
    },
    connections: {
      test: async () => ({ ok: true, value: { summary: 'ok' } }),
      listClusters: async () => ({ ok: true, value: [] }),
      save: async () => 'id',
      delete: async () => {},
      hasSecret: async () => true,
      pickCaFile: async () => null,
      readCaFile: async (path) => ({ path, subject: '', validTo: '', expired: false, count: 1 }),
      importKubeconfig: async () => null,
    },
    databases: {
      list: async () => [],
      test: async () => ({ ok: true }),
      save: async () => 'db-1',
      delete: async () => {},
    },
    sources: { browse: async () => ({ ok: true, value: [] }) },
    streams: { open: async () => 'id', close: async () => {}, onEvent: () => () => {} },
    s3: {
      list: async () => ({ ok: true, value: { entries: [] } }),
      preview: async () => ({ ok: true, value: { kind: 'binary', size: 0, truncated: false } }),
      count: async () => ({ ok: true, value: 0 }),
      download: async () => null,
      upload: async () => null,
      rename: async () => 'op',
      delete: async () => 'op',
      cancel: async () => {},
      onProgress: () => () => {},
      pathForFile: () => '',
    },
    copyText: async () => {},
    services: {
      list: async () => [],
      save: async () => [],
      setFlags: async () => [],
      setToken: async () => ({ ok: true }),
      test: async () => ({ ok: true }),
      forgetToken: async () => [],
      openTokenPage: async () => {},
    },
    actions: {
      list: async () => ({ entries: [], total: 0 }),
      openUrl: async () => {},
      onAdded: () => () => {},
    },
    notifications: {
      status: async () => ({
        enabled: false,
        running: false,
        intervalSeconds: 60,
        target: 'self',
        actives: [],
      }),
      test: async () => ({ ok: true }),
      onChanged: () => () => {},
    },
    contexte: {
      status: async () => ({
        enabled: false,
        running: false,
        intervalSeconds: 60,
        sources: [],
        entities: 0,
      }),
      detectLlm: async () => null,
      agents: async () => [],
      configureAgent: async () => [],
      onChanged: () => () => {},
    },
    mcp: {
      status: async () => ({ enabled: false, running: false, url: '', token: '', executable: '' }),
      regenerateToken: async () => ({
        enabled: false,
        running: false,
        url: '',
        token: '',
        executable: '',
      }),
    },
    logError: async () => {},
    licence: {
      state: async () => ({ status: 'active' as const }),
      activate: async () => ({ ok: true as const, value: { status: 'active' as const } }),
      retry: async () => ({ status: 'active' as const }),
      onChanged: () => () => {},
    },
    analytics: {
      page: async () => {},
      event: async () => {},
      status: async () => ({ enabled: false }),
    },
    readClipboard: async () => '',
    export: { pick: async () => null, write: async () => {}, downloadLogs: async () => null },
    showConfigFile: async () => {},
    showLogFile: async () => {},
  };
}

describe('App', () => {
  beforeEach(() => {
    // jsdom n'implémente pas matchMedia : OS simulé en thème clair.
    window.matchMedia = () => ({ matches: false, addEventListener: () => {} }) as unknown as MediaQueryList;
    window.ocekour = fakeApi();
    document.body.innerHTML = '';
  });

  const settle = async () => {
    for (let i = 0; i < 5; i++) await tick();
    await new Promise((r) => setTimeout(r));
    flushSync();
  };

  it('bascule le thème sombre sur <html>', async () => {
    startServices();
    const app = mount(App, { target: document.body });
    await settle();
    const buttons = document.querySelectorAll('[role=radio]') as NodeListOf<HTMLElement>;

    buttons[2].click(); // Sombre
    await settle();
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(buttons[2].getAttribute('aria-checked')).toBe('true');

    buttons[1].click(); // Clair
    await settle();
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    unmount(app);
  });

  it('navigation : Paramètres puis Actions de l’agent, accueil sans onglet', async () => {
    startServices();
    const app = mount(App, { target: document.body });
    await settle();
    expect(document.body.textContent).toContain('Sélectionnez un conteneur, un topic ou un bucket.');
    const nav = [...document.querySelectorAll('nav button')] as HTMLElement[];
    nav[1].click();
    await settle();
    expect(nav[1].getAttribute('aria-current')).toBe('page');
    nav[2].click();
    await settle();
    expect(nav[2].getAttribute('aria-current')).toBe('page');
    expect(document.body.textContent).not.toContain('Sélectionnez un conteneur');
    unmount(app);
  });
});
