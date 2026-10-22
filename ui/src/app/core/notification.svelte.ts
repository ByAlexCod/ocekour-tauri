export interface Toast {
  id: number;
  kind: 'info' | 'error';
  text: string;
}

/** Notifications éphémères en bas à droite. */
class NotificationService {
  private nextId = 0;
  toasts = $state<Toast[]>([]);

  info(text: string, duration = 3_000) {
    this.push('info', text, duration);
  }

  error(error: unknown) {
    const message = errorMessage(error);
    this.push('error', message, 8_000);
    void window.ocekour?.logError(message).catch(() => undefined);
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  private push(kind: Toast['kind'], text: string, duration: number) {
    const id = this.nextId++;
    this.toasts = [...this.toasts, { id, kind, text }];
    setTimeout(() => this.dismiss(id), duration);
  }
}

export const notifications = new NotificationService();

/** Message lisible d'une erreur (sans préfixe technique). */
export function errorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '');
}
