import type { Theme } from '../../../shared/api';
import { configService } from './config.svelte';

class ThemeService {
  private systemDark = $state(false);
  theme = $derived<Theme>(configService.config?.theme ?? 'system');
  dark = $derived(this.theme === 'dark' || (this.theme === 'system' && this.systemDark));

  start() {
    const media = matchMedia('(prefers-color-scheme: dark)');
    this.systemDark = media.matches;
    media.addEventListener('change', (e) => (this.systemDark = e.matches));
    $effect.root(() => {
      $effect(() => {
        document.documentElement.classList.toggle('dark', this.dark);
      });
    });
  }

  /** Persiste dans config.yaml. */
  set(theme: Theme) {
    void configService.setPreferences({ theme });
  }
}

export const themeService = new ThemeService();
