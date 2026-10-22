import type { SourceErrorInfo, SourceNode, SourcePath } from '../../../shared/api';

export interface NodeState {
  loading: boolean;
  error?: SourceErrorInfo;
  children?: SourceNode[];
  loadedAt?: number;
}

/** Clé d'un nœud : `connexion/cluster/namespace/pod`. */
export const nodeKey = (target: SourcePath) => [target.connectionId, ...target.path].join('/');

export const parseKey = (key: string): SourcePath => {
  const [connectionId, ...path] = key.split('/');
  return { connectionId, path };
};
