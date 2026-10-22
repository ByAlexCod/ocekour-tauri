import { computed, effect, signal, untracked } from '../core/signals.svelte';
import type { StreamOptions } from '../../../shared/api';
import { configService } from '../core/config.svelte';
import { licence } from '../core/licence.svelte';
import { ExplorerSession } from '../explorer/explorer-session';
import { AssemblySession } from './assembly-session';
import { LogSession } from './log-session';
import { resolveProfile } from './profiles';
import { streamService } from './stream.service';
import type { TabState } from './tab-state';
import {
  ASSEMBLY_COLORS,
  type AssemblyMember,
  type AssemblyTarget,
  MAX_ASSEMBLY_MEMBERS,
  migrateTarget,
  profileKey,
  type StreamTarget,
  streamLabel,
  type TabTarget,
  targetKey,
} from './target';

const STORAGE_KEY = 'ocekour.tabs';

/** Un onglet : un flux (conteneur, topic), un assemblage, ou un explorateur (bucket). */
export type Tab = LogSession | ExplorerSession | AssemblySession;

export type { TabState } from './tab-state';

export class TabsService {
  readonly tabs = signal<Tab[]>([]);
  readonly active = signal<Tab | undefined>(undefined);
  readonly activeTarget = computed(() => this.active()?.target);
  private readonly restored = signal(false);

  private started = false;

  /** Effets de la barre d'onglets (visibilité, restauration, sauvegarde). */
  start() {
    if (this.started) return;
    this.started = true;
    // Seul l'onglet affiché est « visible » : les autres comptent leurs nouvelles erreurs.
    effect(() => {
      const active = this.active();
      for (const tab of this.tabs()) {
        const visible = tab === active;
        if (tab.visible() && !visible) tab.hiddenSince = Date.now();
        tab.visible.set(visible);
      }
      untracked(() => active?.unseenErrors.set(0));
    });

    // Restauration au démarrage, une fois la configuration connue.
    effect(() => {
      const config = configService.config;
      // Pas avant l'activation : les flux seraient refusés.
      if (!config || this.restored() || !licence.active) return;
      untracked(() => {
        if (config.restoreTabs) this.restore();
        this.restored.set(true);
      });
    });

    // Sauvegarde de la session (onglets, options, filtres) à chaque changement.
    effect(() => {
      // Pas avant la restauration : on écraserait la session précédente.
      if (!this.restored()) return;
      const tabs = this.tabs();
      const stored = tabs.map((t) => this.stateOf(t));
      const activeIndex = tabs.indexOf(this.active()!);
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify({ tabs: stored, activeIndex }));
      } catch {
        // stockage indisponible : les onglets ne seront pas rouverts
      }
    });
  }

  /**
   * Ouvre une cible : dans un nouvel onglet, ou à la place de l'onglet courant.
   * Si elle est déjà ouverte, son onglet est simplement affiché.
   */
  open(target: TabTarget, newTab = false) {
    const key = targetKey(target);
    const existing = this.tabs().find((t) => targetKey(t.target) === key);
    if (existing) {
      this.active.set(existing);
      return;
    }
    const tab = this.create(target);
    if (!tab) return;
    const current = this.active();
    if (!newTab && current) this.replace(current, tab);
    else this.tabs.update((tabs) => [...tabs, tab]);
    this.active.set(tab);
  }

  /** Remplace la cible d'un onglet de flux (pod de remplacement, par exemple). */
  replaceTarget(tab: LogSession, target: TabTarget) {
    const next = this.create(target, tab.options());
    if (!next) return;
    this.replace(tab, next);
    this.active.set(next);
  }

  /** Ouvre un onglet dans l'état donné (lien de partage) ; affiche l'existant s'il y a. */
  openState(state: TabState) {
    const key = targetKey(state.target);
    const existing = this.tabs().find((t) => targetKey(t.target) === key);
    if (existing) this.close(existing);
    const tab = this.build(state);
    if (!tab) return;
    this.tabs.update((tabs) => [...tabs, tab]);
    this.active.set(tab);
  }

  stateOf(tab: Tab): TabState {
    if (tab instanceof ExplorerSession) return { target: tab.target, prefix: tab.prefix() };
    const base: TabState = {
      target: tab.target,
      options: tab.options(),
      search: tab.search(),
      levels: [...tab.levels()],
      searchMode: tab.searchMode(),
      rules: tab.rules(),
    };
    if (tab instanceof AssemblySession) {
      base.hiddenSources = [...tab.hiddenSources()];
      base.reorderMs = tab.reorderMs();
    }
    return base;
  }

  /** Crée un assemblage vide (actif) : on y ajoute ensuite des sources par clic. */
  startAssembly() {
    const target: AssemblyTarget = { kind: 'assembly', id: randomId(), members: [] };
    const tab = this.create(target);
    if (!tab) return;
    this.tabs.update((tabs) => [...tabs, tab]);
    this.active.set(tab);
  }

  /** Ajoute une source de flux à l'assemblage actif (si c'en est un). Renvoie `true` si ajouté. */
  addToAssembly(target: StreamTarget): boolean {
    const active = this.active();
    if (!(active instanceof AssemblySession)) return false;
    if (active.members().length >= MAX_ASSEMBLY_MEMBERS) return false;
    const key = targetKey(target);
    if (active.members().some((m) => targetKey(m.target) === key)) return true;
    void active.addMember(this.memberOf(target, active.members().length));
    return true;
  }

  private memberOf(target: StreamTarget, index: number): AssemblyMember {
    return {
      id: randomId(),
      target,
      label: streamLabel(target),
      color: ASSEMBLY_COLORS[index % ASSEMBLY_COLORS.length],
    };
  }

  close(tab: Tab | undefined = this.active()) {
    if (!tab) return;
    const tabs = this.tabs();
    const index = tabs.indexOf(tab);
    this.dispose(tab);
    const rest = tabs.filter((t) => t !== tab);
    this.tabs.set(rest);
    if (this.active() === tab) this.active.set(rest[Math.min(index, rest.length - 1)]);
  }

  /** Ctrl+Tab / Ctrl+Maj+Tab. */
  cycle(direction: 1 | -1) {
    const tabs = this.tabs();
    if (tabs.length < 2) return;
    const index = tabs.indexOf(this.active()!);
    this.active.set(tabs[(index + direction + tabs.length) % tabs.length]);
  }

  move(from: number, to: number) {
    this.tabs.update((tabs) => {
      const next = [...tabs];
      next.splice(to, 0, ...next.splice(from, 1));
      return next;
    });
  }

  private create(target: TabTarget, options?: StreamOptions): Tab | undefined {
    if (target.kind === 'bucket') return new ExplorerSession(target);
    if (target.kind === 'assembly') return this.createAssembly(target, options);
    return streamService.open(target, options);
  }

  private createAssembly(target: AssemblyTarget, options?: StreamOptions): Tab | undefined {
    const api = streamService.api;
    if (!api) return undefined;
    const config = configService.config;
    const session = new AssemblySession(
      target,
      { follow: true, tailLines: config?.defaultTailLines ?? 1_000, ...options },
      config?.bufferSize ?? 100_000,
      { api, bind: (id, s) => streamService.bind(id, s), unbind: (id) => streamService.unbind(id) },
      (t) => resolveProfile(configService.config?.profiles ?? [], profileKey(t)).fields,
    );
    void session.start();
    return session;
  }

  private build(state: TabState): Tab | undefined {
    const tab = this.create(state.target, state.options);
    if (tab instanceof ExplorerSession) {
      tab.prefix.set(state.prefix ?? '');
    } else if (tab) {
      if (state.search) tab.search.set(state.search);
      if (state.levels) tab.levels.set(new Set(state.levels));
      if (state.searchMode) tab.searchMode.set(state.searchMode);
      if (state.rules) tab.rules.set(state.rules);
      if (tab instanceof AssemblySession) {
        if (state.hiddenSources) tab.hiddenSources.set(new Set(state.hiddenSources));
        if (state.reorderMs !== undefined) tab.reorderMs.set(state.reorderMs);
      }
    }
    return tab;
  }

  private dispose(tab: Tab) {
    if (tab instanceof LogSession) streamService.close(tab);
    else tab.dispose();
  }

  private replace(old: Tab, tab: Tab) {
    this.dispose(old);
    this.tabs.update((tabs) => tabs.map((t) => (t === old ? tab : t)));
  }

  private restore() {
    let saved: { tabs: TabState[]; activeIndex: number };
    try {
      saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null');
    } catch {
      return;
    }
    if (!saved?.tabs?.length) return;
    const connections = new Set(configService.config?.connections.map((c) => c.id));
    const tabs = saved.tabs
      .map((stored) => ({ ...stored, target: migrateTarget(stored.target) }))
      .map((stored) => this.keepReachable(stored, connections))
      .filter((stored): stored is TabState => !!stored)
      .map((stored) => this.build(stored))
      .filter((t): t is Tab => !!t);
    this.tabs.set(tabs);
    this.active.set(tabs[Math.min(Math.max(saved.activeIndex, 0), tabs.length - 1)]);
  }

  /**
   * Garde un onglet dont la (les) connexion(s) existe(nt) encore. Pour un assemblage, on ne
   * garde que les membres joignables ; l'onglet est abandonné s'il n'en reste aucun.
   */
  private keepReachable(state: TabState, connections: Set<string>): TabState | undefined {
    if (state.target.kind === 'assembly') {
      const members = state.target.members.filter((m) => connections.has(m.target.connectionId));
      if (!members.length) return undefined;
      return { ...state, target: { ...state.target, members } };
    }
    return connections.has(state.target.connectionId) ? state : undefined;
  }
}

/** Identifiant court et unique (assemblage, membre). */
function randomId(): string {
  return Math.random().toString(36).slice(2, 10);
}

export const tabsService = new TabsService();
