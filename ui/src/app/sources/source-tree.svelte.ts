import { LOCAL_CONNECTION, type SourcePath } from '../../../shared/api';
import { configService } from '../core/config.svelte';
import { licence } from '../core/licence.svelte';
import { type NodeState, nodeKey, parseKey } from './source-tree.service';

const STORAGE_KEY = 'ocekour.tree.expanded';
/** Au-delà, un nœud déplié à nouveau est rechargé (les pods bougent vite). */
const STALE_MS = 30_000;

/**
 * État de l'arborescence : enfants chargés paresseusement, nœuds dépliés (mémorisés d'une
 * session à l'autre).
 */
class SourceTreeService {
  nodes = $state.raw<ReadonlyMap<string, NodeState>>(new Map());
  /** Nœuds dépliés, sauf pour les connexions où l'ensemble contient les connexions repliées. */
  toggled = $state.raw<ReadonlySet<string>>(readStored());
  private started = false;

  private get api() {
    return window.ocekour?.sources;
  }

  start() {
    if (this.started) return;
    this.started = true;
    $effect.root(() => {
      $effect(() => {
        const toggled = this.toggled;
        try {
          localStorage.setItem(STORAGE_KEY, JSON.stringify([...toggled]));
        } catch {
          // Stockage indisponible : l'arbre repartira replié, sans gravité.
        }
      });
      // Recharge les nœuds dépliés lors de la session précédente, une fois la config connue.
      $effect(() => {
        const connections = configService.config?.connections;
        // Rien avant l'activation (les sources refusent de répondre).
        if (!connections || !licence.active) return;
        const toggled = this.toggled;
        const nodes = this.nodes;
        const ids = new Set([LOCAL_CONNECTION, ...connections.map((c) => c.id)]);
        for (const c of connections) {
          const root = { connectionId: c.id, path: [] };
          if (c.type !== 'rancher' && this.isExpanded(root) && !nodes.has(c.id)) void this.load(root);
        }
        for (const stored of toggled) {
          // Favori déplié : c'est son namespace qu'il faut charger.
          const key = stored.startsWith('★') ? stored.slice(1).split('/').slice(0, 3).join('/') : stored;
          const target = parseKey(key);
          if (target.path.length && ids.has(target.connectionId) && !nodes.has(key)) void this.load(target);
        }
      });
    });
  }

  isExpanded(target: SourcePath): boolean {
    const toggled = this.toggled.has(nodeKey(target));
    return target.path.length ? toggled : !toggled;
  }

  /** Favoris dépliés : mémorisés avec l'arbre, sous la clé `★<favori>`. */
  isFavoriteOpen(favorite: string): boolean {
    return this.toggled.has(`★${favorite}`);
  }

  /** Déplie / replie un favori ; charge son namespace si besoin. */
  toggleFavoriteOpen(favorite: string, namespace: SourcePath) {
    const key = `★${favorite}`;
    const opening = !this.toggled.has(key);
    const next = new Set(this.toggled);
    if (opening) next.add(key);
    else next.delete(key);
    this.toggled = next;
    if (opening) this.loadIfStale(namespace);
  }

  /** Racine Kafka / S3 : ses enfants (topics, buckets) viennent du réseau, pas de la config. */
  private isRemoteRoot(target: SourcePath): boolean {
    const connection = configService.config?.connections.find((c) => c.id === target.connectionId);
    return !target.path.length && !!connection && connection.type !== 'rancher';
  }

  private loadIfStale(target: SourcePath) {
    const state = this.nodes.get(nodeKey(target));
    if (!state || (!state.loading && Date.now() - (state.loadedAt ?? 0) > STALE_MS)) void this.load(target);
  }

  /** `localOnly` : dépliage purement visuel (groupe de topics), sans chargement. */
  toggle(target: SourcePath, localOnly = false) {
    const key = nodeKey(target);
    const expanding = !this.isExpanded(target);
    const next = new Set(this.toggled);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    this.toggled = next;
    if (expanding && !localOnly && (target.path.length || this.isRemoteRoot(target))) this.loadIfStale(target);
  }

  async load(target: SourcePath) {
    if (!this.api) return;
    const key = nodeKey(target);
    this.patch(key, { ...this.nodes.get(key), loading: true, error: undefined });
    const result = await this.api.browse(target);
    this.patch(
      key,
      result.ok
        ? { loading: false, children: result.value, loadedAt: Date.now() }
        : { loading: false, error: result.error, children: this.nodes.get(key)?.children },
    );
  }

  /** Recharge tous les nœuds dépliés et chargés. */
  refreshAll() {
    for (const key of this.nodes.keys()) {
      const target = parseKey(key);
      if (this.isExpanded(target)) void this.load(target);
    }
  }

  private patch(key: string, state: NodeState) {
    this.nodes = new Map(this.nodes).set(key, state);
  }
}

function readStored(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]'));
  } catch {
    return new Set();
  }
}

export const sourceTree = new SourceTreeService();
