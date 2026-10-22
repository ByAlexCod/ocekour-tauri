export interface TrackedEvent {
  category: string;
  action: string;
  name?: string;
  value?: number;
}

/**
 * Statistiques d'utilisation anonymes (Matomo, via le cœur). N'envoyer que des libellés
 * fixes : jamais de nom de serveur, cluster, pod, topic, bucket ni de texte saisi.
 */
class AnalyticsService {
  private lastPage = '';

  page(title: string, path: string) {
    if (path === this.lastPage) return;
    this.lastPage = path;
    void window.ocekour?.analytics.page(title, path).catch(() => undefined);
  }

  event(event: TrackedEvent) {
    void window.ocekour?.analytics.event(event).catch(() => undefined);
  }
}

export const analytics = new AnalyticsService();
