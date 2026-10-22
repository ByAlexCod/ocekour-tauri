import type { ConfigState, DeepPartial, NotificationsConfig, OcekourApi, Preferences, StoredFilterRule } from '../../../shared/api';
import { notifications } from './notification.svelte';

declare global {
  interface Window {
    // Absent quand l'interface tourne hors Tauri (tests, navigateur).
    ocekour?: OcekourApi;
  }
}

class ConfigService {
  /** `null` tant que la config n'est pas chargée (ou hors application). */
  state = $state<ConfigState | null>(null);
  config = $derived(this.state?.config);
  errors = $derived(this.state?.errors ?? []);
  /** Filtres de logs enregistrés (partagés par tous les onglets). */
  filterPresets = $derived(this.config?.filterPresets ?? []);
  private started = false;

  get api() {
    return window.ocekour?.config;
  }

  /** Chargement initial et suivi des modifications (appelé au démarrage). */
  start() {
    if (this.started) return;
    this.started = true;
    void this.api?.get().then((s) => (this.state = s));
    this.api?.onChanged((s) => (this.state = s));
  }

  showLogFile() {
    void window.ocekour?.showLogFile();
  }

  showFile() {
    void window.ocekour?.showConfigFile();
  }

  private async apply(run: () => Promise<ConfigState> | undefined) {
    try {
      const next = await run();
      if (next) this.state = next;
    } catch (e) {
      notifications.error(e);
    }
  }

  setPreferences(patch: Partial<Preferences>) {
    return this.apply(() => this.api?.setPreferences(patch));
  }

  setNotifications(patch: DeepPartial<NotificationsConfig>) {
    return this.apply(() => this.api?.setNotifications(patch));
  }

  saveFilterPreset(name: string, rules: StoredFilterRule[]) {
    return this.apply(() => this.api?.saveFilterPreset(name, rules));
  }

  deleteFilterPreset(name: string) {
    return this.apply(() => this.api?.deleteFilterPreset(name));
  }

  renameFilterPreset(name: string, to: string) {
    return this.apply(() => this.api?.renameFilterPreset(name, to));
  }
}

export const configService = new ConfigService();
