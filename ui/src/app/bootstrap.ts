import { actionsService } from './actions/actions.service';
import { configService } from './core/config.svelte';
import { licence } from './core/licence.svelte';
import { effect } from './core/signals.svelte';
import { localFiles } from './files/local-files.svelte';
import { themeService } from './core/theme.svelte';
import { streamService } from './logs/stream.service';
import { tabsService } from './logs/tabs.service';
import { shareService } from './share/share.service';
import { sourceTree } from './sources/source-tree.svelte';

/**
 * Démarre les services de l'interface (abonnements au cœur, effets globaux). Remplace
 * l'injection de dépendances d'Angular : chaque service est un singleton démarré une fois.
 */
export function startServices() {
  configService.start();
  licence.start();
  themeService.start();
  sourceTree.start();
  streamService.start();
  tabsService.start();
  shareService.start();
  actionsService.start();
  localFiles.start();
  // Pastille « Actions de l'agent » : comptée une fois l'application activée.
  effect(() => {
    if (licence.active) actionsService.refresh();
  });
}
