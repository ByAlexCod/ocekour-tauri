import { signal } from '../core/signals.svelte';
import type { AgentAction } from '../../../shared/api';

const SEEN_KEY = 'ocekour.actions.seen';

const readSeen = (): string => {
  try {
    return localStorage.getItem(SEEN_KEY) ?? '';
  } catch {
    return '';
  }
};

/** Nouvelles actions depuis la dernière visite de la page (pastille du menu). */
class ActionsService {
  readonly unseen = signal(0);
  /** Dernière action ajoutée (rafraîchit la page ouverte). */
  readonly last = signal<AgentAction | undefined>(undefined);

  /** Compte les actions non vues (appelé une fois l'application activée). */
  refresh() {
    const seen = readSeen();
    window.ocekour?.actions
      .list({ limit: 1_000 })
      .then(({ entries }) => this.unseen.set(entries.filter((a) => a.date > seen).length))
      .catch(() => undefined);
  }

  private started = false;

  start() {
    if (this.started) return;
    this.started = true;
    window.ocekour?.actions.onAdded((action) => {
      this.unseen.update((n) => n + 1);
      this.last.set(action);
    });
  }

  markSeen() {
    try {
      localStorage.setItem(SEEN_KEY, new Date().toISOString());
    } catch {
      // stockage indisponible : la pastille repartira de zéro au prochain lancement
    }
    this.unseen.set(0);
  }
}

export const actionsService = new ActionsService();
