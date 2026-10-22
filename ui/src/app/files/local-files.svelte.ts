import type { FileEncoding, LocalFile } from '../../../shared/api';
import { notifications } from '../core/notification.svelte';
import { LogSession } from '../logs/log-session';
import { tabsService as tabs } from '../logs/tabs.service';
import { baseName, type FileTarget, targetKey } from '../logs/target';

/** Le chemin désigne un motif (jokers dans le nom du fichier). */
export function isPattern(path: string): boolean {
  return /[*?[]/.test(baseName(path));
}

/** Onglet d'un fichier du poste (ou d'un fichier choisi dans un motif). */
export function localTarget(path: string, options: { file?: string; encoding?: FileEncoding } = {}): FileTarget {
  const pattern = isPattern(path);
  return {
    kind: 'file',
    path: options.file ? [path, options.file] : [path],
    name: baseName(path),
    pattern: pattern && !options.file,
    encoding: options.encoding,
  };
}

/**
 * Section « Fichiers locaux » : fichiers récents et favoris (propres au poste), ouverture par
 * la boîte de dialogue, Ctrl+O, glisser-déposer ou ligne de commande.
 */
class LocalFilesService {
  list = $state.raw<LocalFile[]>([]);
  private started = false;

  private get api() {
    return window.ocekour?.files;
  }

  start() {
    if (this.started) return;
    this.started = true;
    void this.refresh();
    this.api?.onOpen((paths) => this.openPaths(paths, true));
    void this.api?.pending().then((paths) => this.openPaths(paths, true)).catch(() => {});
    // Fichiers déposés sur la fenêtre : ouverts en onglets, sauf sur l'explorateur S3
    // (qui les envoie dans le bucket).
    this.api?.onDropped((paths) => {
      if (tabs.active()?.kind === 'explorer') return;
      this.openPaths(paths.filter((p) => !/[\\/]$/.test(p)), tabs.tabs().length > 0);
    });
  }

  async refresh() {
    try {
      this.list = (await this.api?.list()) ?? [];
    } catch {
      // liste inchangée
    }
  }

  private async update(run: () => Promise<LocalFile[]> | undefined) {
    try {
      const next = await run();
      if (next) this.list = next;
    } catch (e) {
      notifications.error(e);
    }
  }

  encodingOf(path: string): FileEncoding | undefined {
    return this.list.find((f) => f.path === path)?.encoding;
  }

  /** Ouvre un fichier ou un motif (récents mis à jour). */
  open(path: string, newTab = false, file?: string) {
    void this.update(() => this.api?.touch(path));
    tabs.open(localTarget(path, { file, encoding: this.encodingOf(path) }), newTab);
  }

  /** Plusieurs fichiers : le premier dans l'onglet courant (sauf `newTab`), les autres à côté. */
  openPaths(paths: string[], newTab = false) {
    paths.forEach((p, i) => this.open(p, newTab || i > 0));
  }

  async pick(newTab = false) {
    try {
      const paths = (await this.api?.pick()) ?? [];
      this.openPaths(paths, newTab || paths.length > 1);
    } catch (e) {
      notifications.error(e);
    }
  }

  favorite(path: string, favorite: boolean) {
    void this.update(() => this.api?.favorite(path, favorite));
  }

  forget(path: string) {
    void this.update(() => this.api?.forget(path));
  }

  /** Change l'encodage (mémorisé pour ce chemin) et rouvre l'onglet avec. */
  setEncoding(target: FileTarget, encoding: FileEncoding | undefined) {
    const path = target.path[0];
    void this.update(() => this.api?.setEncoding(path, encoding ?? null));
    const tab = tabs.tabs().find((t) => targetKey(t.target) === targetKey(target));
    if (tab instanceof LogSession) tabs.replaceTarget(tab, { ...target, encoding });
  }
}

export const localFiles = new LocalFilesService();
