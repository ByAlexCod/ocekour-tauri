import { signal } from '../core/signals.svelte';
import type { Connection, ConnectionType } from '../../../shared/api';

/** Valeurs proposées à la création d'une connexion (lien de partage). */
export interface ConnectionPrefill {
  type: ConnectionType;
  url?: string;
  brokers?: string[];
  /** Cluster Rancher à cocher. */
  cluster?: { id: string; name: string };
}

export interface ConnectionDialogRequest {
  connection: Connection | null;
  focusToken: boolean;
  prefill?: ConnectionPrefill;
}

/** Ouverture de la fenêtre de connexion depuis n'importe où (arbre, lien collé). */
class ConnectionDialogService {
  readonly request = signal<ConnectionDialogRequest | null>(null);

  open(
    connection: Connection | null,
    options: Omit<ConnectionDialogRequest, 'connection'> = { focusToken: false },
  ) {
    this.request.set({ connection, ...options });
  }

  close() {
    this.request.set(null);
  }
}

export const connectionDialog = new ConnectionDialogService();
