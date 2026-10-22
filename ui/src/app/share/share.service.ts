import { effect, signal, untracked } from '../core/signals.svelte';
import type { Connection } from '../../../shared/api';
import { analytics } from '../core/analytics';
import { connectionDialog } from '../connections/connection-dialog.service';
import { copy } from '../core/clipboard';
import { configService } from '../core/config.svelte';
import { notifications } from '../core/notification.svelte';
import { type Tab, tabsService } from '../logs/tabs.service';
import {
  decodeLink,
  encodeLink,
  findConnection,
  linkFor,
  type ShareLink,
  stateFor,
} from './share-link';

/** Copier le lien d'un onglet ; ouvrir un lien collé (Ctrl+V ou bouton). */
class ShareService {
  /** Lien en attente d'une connexion (créée ou complétée dans la fenêtre de connexion). */
  private readonly pending = signal<ShareLink | null>(null);

  private started = false;

  start() {
    if (this.started) return;
    this.started = true;
    // Connexion créée ou cluster ajouté : le lien en attente s'ouvre de lui-même.
    effect(() => {
      const connections = configService.config?.connections ?? [];
      const link = this.pending();
      if (!link) return;
      untracked(() => {
        const connection = findConnection(link, connections);
        if (connection && !this.missingCluster(link, connection)) {
          this.pending.set(null);
          tabsService.openState(stateFor(link, connection));
        }
      });
    });
  }

  copyLink(tab: Tab) {
    const target = tab.target;
    // L'assemblage (plusieurs sources) n'est pas partageable par lien dans cette version.
    if (target.kind === 'assembly') return;
    const connection = this.connections().find((c) => c.id === target.connectionId);
    if (!connection) return;
    const link = encodeLink(linkFor(tabsService.stateOf(tab), connection));
    analytics.event({ category: 'Partage', action: 'Lien copié', name: connection.type });
    void copy(link, 'Lien copié : à coller (Ctrl+V) dans Ocekour');
  }

  /** Bouton « Ouvrir un lien » : lit le presse-papiers. */
  async openFromClipboard() {
    const text = (await window.ocekour?.readClipboard()) ?? '';
    this.open(text);
  }

  open(text: string) {
    let link: ShareLink;
    try {
      link = decodeLink(text);
    } catch (e) {
      notifications.error(e);
      analytics.event({ category: 'Partage', action: 'Lien ouvert', name: 'invalide' });
      return;
    }
    const connection = findConnection(link, this.connections());
    const opened = (result: string) =>
      analytics.event({ category: 'Partage', action: 'Lien ouvert', name: result });
    if (!connection) {
      const where = link.t === 'kafka' ? link.brokers.join(', ') : link.url;
      if (!confirm(`Aucune connexion ne correspond à ${where}.\nLa créer maintenant ?`)) return;
      opened(`connexion à créer (${link.t})`);
      this.pending.set(link);
      connectionDialog.open(null, {
        focusToken: false,
        prefill: {
          type: link.t,
          url: link.t === 'kafka' ? undefined : link.url,
          brokers: link.t === 'kafka' ? link.brokers : undefined,
          cluster:
            link.t === 'rancher' ? { id: link.clusterId, name: link.clusterName } : undefined,
        },
      });
      return;
    }
    if (link.t === 'rancher' && this.missingCluster(link, connection)) {
      const question = `Le cluster ${link.clusterName} n'est pas coché dans « ${connection.name} ».\nL'ajouter ?`;
      if (!confirm(question)) return;
      opened('cluster à ajouter');
      this.pending.set(link);
      connectionDialog.open(connection, {
        focusToken: false,
        prefill: { type: 'rancher', cluster: { id: link.clusterId, name: link.clusterName } },
      });
      return;
    }
    opened(link.t);
    tabsService.openState(stateFor(link, connection));
  }

  private missingCluster(link: ShareLink, connection: Connection): boolean {
    return (
      link.t === 'rancher' &&
      connection.type === 'rancher' &&
      !connection.clusters.some((c) => c.id === link.clusterId)
    );
  }

  private connections(): Connection[] {
    return configService.config?.connections ?? [];
  }
}

export const shareService = new ShareService();
