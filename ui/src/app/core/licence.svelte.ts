import type { LicenceState } from '../../../shared/api';
import { errorMessage, notifications } from './notification.svelte';

/** Clé d'activation : état reçu du cœur (l'application n'est utilisable qu'activée). */
class LicenceService {
  /** Hors application (tests, navigateur), l'application est considérée comme active. */
  state = $state<LicenceState>({ status: 'checking' });
  active = $derived(this.state.status === 'active');
  /** « Changer de clé » demandé depuis les Paramètres. */
  changing = $state(false);

  private get api() {
    return window.ocekour?.licence;
  }

  start() {
    if (!this.api) {
      this.state = { status: 'active' };
      return;
    }
    void this.api.state().then((s) => (this.state = s));
    this.api.onChanged((s) => (this.state = s));
  }

  /** Renvoie le message d'erreur si la clé est refusée. */
  async activate(key: string): Promise<string | undefined> {
    if (!this.api) return undefined;
    try {
      const result = await this.api.activate(key);
      if (!result.ok) return result.error.message;
      this.state = result.value;
      this.changing = false;
      const preset = result.value.preset;
      if (preset?.connections.length) {
        notifications.info(
          `Clé activée. Connexion(s) ajoutée(s) : ${preset.connections.join(', ')}. ` +
            'Saisissez vos identifiants via « Mettre à jour les identifiants » dans l’arbre.',
          15_000,
        );
      } else if (preset?.profiles) {
        notifications.info(`Clé activée. ${preset.profiles} profil(s) ajouté(s).`, 8_000);
      }
      return undefined;
    } catch (e) {
      return errorMessage(e);
    }
  }

  async retry() {
    if (this.api) this.state = await this.api.retry();
  }
}

export const licence = new LicenceService();
