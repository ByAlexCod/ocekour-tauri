import type { StreamOptions } from '../../../shared/api';
import type { SearchOptions } from './filter';
import type { LogFilterRule } from './log-filters';
import type { Level } from './parser';
import type { TabTarget } from './target';

/** Ce qui est mémorisé d'un onglet pour le rouvrir (démarrage, lien de partage). */
export interface TabState {
  target: TabTarget;
  options?: StreamOptions;
  search?: SearchOptions;
  levels?: Level[];
  searchMode?: 'filter' | 'highlight';
  /** Règles de filtre (masquer / garder seul). */
  rules?: LogFilterRule[];
  prefix?: string;
  /** Assemblage : sources masquées (ids de membres). */
  hiddenSources?: string[];
  /** Assemblage : fenêtre de réordonnancement en direct (ms). */
  reorderMs?: number;
}
