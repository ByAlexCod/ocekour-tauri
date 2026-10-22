import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { OcekourApi } from '../shared/api';

// Pont entre l'interface et le cœur Rust : chaque méthode de `OcekourApi` appelle le canal
// du même nom que la version Electron (`call(canal, arguments)`), et les événements du cœur
// arrivent par `listen`. Le reste de l'interface ne voit que `window.ocekour`.

const call = <T>(channel: string, ...args: unknown[]): Promise<T> =>
  invoke<T>('call', { channel, args }).catch((e: unknown) => {
    throw e instanceof Error ? e : new Error(String(e));
  });

/** Abonnement à un événement du cœur ; renvoie la fonction de désabonnement. */
function on<T>(event: string, listener: (payload: T) => void): () => void {
  let stop: (() => void) | undefined;
  let stopped = false;
  void listen<T>(event, (e) => listener(e.payload)).then((unlisten) => {
    if (stopped) unlisten();
    else stop = unlisten;
  });
  return () => {
    stopped = true;
    stop?.();
  };
}

/** Chemins des derniers fichiers déposés sur la fenêtre (glisser-déposer depuis l'Explorateur). */
let dropped: string[] = [];
const dropListeners = new Set<(paths: string[]) => void>();

export function installDropTracking() {
  void import('@tauri-apps/api/webview').then(({ getCurrentWebview }) =>
    getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type !== 'drop') return;
      dropped = e.payload.paths;
      for (const l of dropListeners) l(dropped);
    }),
  );
}

function onDropped(listener: (paths: string[]) => void): () => void {
  dropListeners.add(listener);
  return () => dropListeners.delete(listener);
}

/** Chemin local d'un fichier déposé : retrouvé par son nom parmi les derniers déposés. */
export function pathForFile(file: File): string {
  const name = file.name;
  return dropped.find((p) => p.split(/[\\/]/).pop() === name) ?? '';
}

export function tauriApi(): OcekourApi {
  return {
    config: {
      get: () => call('config:get'),
      setPreferences: (patch) => call('config:setPreferences', patch),
      setNotifications: (patch) => call('config:setNotifications', patch),
      setContexte: (patch) => call('config:setContexte', patch),
      setFichiers: (patch) => call('config:setFichiers', patch),
      readText: () => call('config:readText'),
      validateText: (text) => call('config:validateText', text),
      saveText: (text) => call('config:saveText', text),
      editProfile: (match, edit) => call('config:editProfile', match, edit),
      saveFilterPreset: (name, rules) => call('config:saveFilterPreset', name, rules),
      deleteFilterPreset: (name) => call('config:deleteFilterPreset', name),
      renameFilterPreset: (name, to) => call('config:renameFilterPreset', name, to),
      toggleFavorite: (key) => call('config:toggleFavorite', key),
      onChanged: (listener) => on('config:changed', listener),
    },
    connections: {
      test: (draft) => call('connections:test', draft),
      listClusters: (draft) => call('connections:listClusters', draft),
      save: (draft) => call('connections:save', draft),
      delete: (id) => call('connections:delete', id),
      hasSecret: (id) => call('connections:hasSecret', id),
      pickCaFile: () => call('connections:pickCaFile'),
      readCaFile: (path) => call('connections:readCaFile', path),
      importKubeconfig: () => call('connections:importKubeconfig'),
    },
    databases: {
      list: () => call('databases:list'),
      test: (draft) => call('databases:test', draft),
      save: (draft) => call('databases:save', draft),
      delete: (id) => call('databases:delete', id),
    },
    sources: { browse: (target) => call('sources:browse', target) },
    files: {
      list: () => call('files:list'),
      pending: () => call('files:pending'),
      pick: () => call('files:pick'),
      touch: (path) => call('files:touch', path),
      favorite: (path, favorite) => call('files:favorite', path, favorite),
      forget: (path) => call('files:forget', path),
      setEncoding: (path, encoding) => call('files:setEncoding', path, encoding),
      onOpen: (listener) => on<string[]>('files:open', listener),
      onDropped,
    },
    sonde: { fingerprint: (url) => call('sonde:fingerprint', url) },
    streams: {
      open: (target, options) => call('streams:open', target, options),
      close: (id) => call('streams:close', id),
      onEvent: (listener) =>
        on<{ id: string; event: Parameters<typeof listener>[1] }>('streams:event', (p) => listener(p.id, p.event)),
    },
    s3: {
      list: (ref, prefix, options) => call('s3:list', ref, prefix, options ?? {}),
      preview: (ref, key) => call('s3:preview', ref, key),
      count: (ref, prefix) => call('s3:count', ref, prefix),
      download: (ref, entries) => call('s3:download', ref, entries),
      upload: (ref, prefix, source) => call('s3:upload', ref, prefix, source),
      rename: (ref, entry, newName) => call('s3:rename', ref, entry, newName),
      delete: (ref, entries) => call('s3:delete', ref, entries),
      cancel: (opId) => call('s3:cancel', opId),
      onProgress: (listener) => on('s3:progress', listener),
      pathForFile,
    },
    copyText: (text) => call('app:copyText', text),
    licence: {
      state: () => call('licence:state'),
      activate: (key) => call('licence:activate', key),
      retry: () => call('licence:retry'),
      onChanged: (listener) => on('licence:changed', listener),
    },
    analytics: {
      page: (title, path) => call('analytics:page', title, path),
      event: (event) => call('analytics:event', event),
      status: () => call('analytics:status'),
    },
    services: {
      list: () => call('services:list'),
      save: (id, settings) => call('services:save', id, settings),
      setFlags: (id, flags) => call('services:setFlags', id, flags),
      setToken: (id, token, userId) => call('services:setToken', id, token, userId),
      test: (id) => call('services:test', id),
      forgetToken: (id) => call('services:forgetToken', id),
      openTokenPage: (id, fallback) => call('services:openTokenPage', id, fallback),
    },
    actions: {
      list: (query) => call('actions:list', query),
      openUrl: (url) => call('actions:openUrl', url),
      onAdded: (listener) => on('actions:added', listener),
    },
    notifications: {
      status: () => call('notifications:status'),
      test: () => call('notifications:test'),
      onChanged: (listener) => on('notifications:changed', listener),
    },
    contexte: {
      status: () => call('contexte:status'),
      detectLlm: () => call('contexte:detectLlm'),
      agents: () => call('contexte:agents'),
      configureAgent: (tool, action) => call('contexte:configureAgent', tool, action),
      onChanged: (listener) => on('contexte:changed', listener),
    },
    mcp: {
      status: () => call('mcp:status'),
      regenerateToken: () => call('mcp:regenerateToken'),
    },
    logError: (message) => call('app:logError', message),
    readClipboard: () => call('app:readClipboard'),
    export: {
      pick: (defaultName) => call('export:pick', defaultName),
      write: (token, content) => call('export:write', token, content),
      downloadLogs: (request) => call('export:downloadLogs', request),
    },
    showConfigFile: () => call('app:showConfigFile'),
    showLogFile: () => call('app:showLogFile'),
  };
}
