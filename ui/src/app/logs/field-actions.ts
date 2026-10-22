import { getContext, setContext } from 'svelte';
import { computed, signal } from '../core/signals.svelte';
import type { ProfileEdit } from '../../../shared/api';
import { analytics } from '../core/analytics';
import { configService } from '../core/config.svelte';
import { notifications } from '../core/notification.svelte';
import type { ResolvedProfile } from './profiles';

export type FieldRole = 'time' | 'level' | 'message';

/**
 * Actions « en un clic » sur les champs d'une ligne, appliquées au profil exact du
 * workload affiché (`cluster/namespace/workload`) dans config.yaml.
 */
export class FieldActions {
  readonly profile = signal<ResolvedProfile | undefined>(undefined);
  /** Dernière demande « filtrer sur cette valeur » (traitée par la vue du flux). */
  readonly filterRequest = signal<{ text: string } | null>(null);

  filterOn(value: unknown) {
    const text = typeof value === 'string' ? value : JSON.stringify(value);
    this.filterRequest.set({ text: text.length > 500 ? text.slice(0, 500) : text });
  }

  private readonly expand = computed(() => new Set(this.profile()?.expand ?? []));
  private readonly columns = computed(() => new Set(this.profile()?.columns ?? []));

  isExpanded(path: string) {
    return this.expand().has(path);
  }

  isColumn(path: string) {
    return this.columns().has(path);
  }

  roleOf(path: string): FieldRole | undefined {
    const fields = this.profile()?.fields ?? {};
    return (['time', 'level', 'message'] as const).find((role) => fields[role] === path);
  }

  toggleExpand(path: string) {
    return this.edit(
      { op: 'toggleExpand', path },
      this.isExpanded(path)
        ? `« ${path} » n'est plus mis en avant`
        : `« ${path} » sera toujours déplié`,
    );
  }

  toggleColumn(path: string) {
    return this.edit(
      { op: 'toggleColumn', path },
      this.isColumn(path) ? `Colonne « ${path} » retirée` : `Colonne « ${path} » ajoutée`,
    );
  }

  isWrapped(path: string): boolean {
    return !!this.profile()?.wrap.includes(path);
  }

  /** JSON de la colonne indenté (plusieurs lignes) ou sur une ligne ; `@message` : message. */
  toggleWrap(path: string) {
    const name = path === '@message' ? 'message' : path;
    return this.edit(
      { op: 'toggleWrap', path },
      this.isWrapped(path)
        ? `Colonne « ${name} » : JSON sur une ligne`
        : `Colonne « ${name} » : JSON indenté`,
    );
  }

  useAs(role: FieldRole, path: string | null) {
    const labels = { time: 'heure', level: 'niveau', message: 'message' };
    return this.edit(
      { op: 'setField', role, path },
      path
        ? `« ${path} » utilisé comme ${labels[role]}`
        : `${labels[role]} : détection automatique`,
    );
  }

  setParseNested(value: boolean) {
    return this.edit(
      { op: 'setParseNested', value },
      value ? 'JSON imbriqué déplié par défaut' : 'JSON imbriqué non déplié par défaut',
    );
  }

  private async edit(edit: ProfileEdit, done: string) {
    const key = this.profile()?.key;
    const api = configService.api;
    if (!key || !api) return;
    try {
      await api.editProfile(key, edit);
      analytics.event({ category: 'Profil', action: edit.op });
      notifications.info(`${done} pour ${key}`);
    } catch (e) {
      notifications.error(e);
    }
  }
}

const KEY = Symbol('actions-de-champ');

/** Actions de champ de la vue du flux (contexte des composants du détail). */
export function provideFieldActions(actions: FieldActions) {
  setContext(KEY, actions);
}

export function fieldActions(): FieldActions | undefined {
  return getContext<FieldActions | undefined>(KEY);
}
