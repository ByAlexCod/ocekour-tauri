import type { StreamEvent, StreamOptions } from '../../../shared/api';
import { configService } from '../core/config.svelte';
import { LogSession } from './log-session';
import type { StreamTarget } from './target';

/** Destinataire des événements d'un flux (session simple ou assemblage). */
export interface StreamConsumer {
  handle(event: StreamEvent, id?: string): void;
}

/** Crée les sessions de logs et leur distribue les événements venant du cœur. */
class StreamService {
  private readonly consumers = new Map<string, StreamConsumer>();
  private started = false;

  get api() {
    return window.ocekour?.streams;
  }

  start() {
    if (this.started) return;
    this.started = true;
    this.api?.onEvent((id, event) => this.consumers.get(id)?.handle(event, id));
  }

  /** Lie un identifiant de flux à son destinataire (assemblage : plusieurs par session). */
  bind(id: string, consumer: StreamConsumer) {
    this.consumers.set(id, consumer);
  }

  unbind(id: string) {
    this.consumers.delete(id);
  }

  open(target: StreamTarget, options?: Partial<StreamOptions>): LogSession | undefined {
    const api = this.api;
    if (!api) return undefined;
    this.start();
    const config = configService.config;
    const session = new LogSession(
      target,
      { follow: true, tailLines: config?.defaultTailLines ?? 1_000, ...options },
      config?.bufferSize ?? 100_000,
      api,
      (id, s) => {
        // L'identifiant précédent (options changées) ne reçoit plus rien.
        for (const [key, value] of this.consumers) if (value === s) this.consumers.delete(key);
        this.consumers.set(id, s);
      },
    );
    void session.start();
    return session;
  }

  close(session: StreamConsumer) {
    for (const [key, value] of this.consumers) if (value === session) this.consumers.delete(key);
    if (session instanceof LogSession) void session.dispose();
  }
}

export const streamService = new StreamService();
