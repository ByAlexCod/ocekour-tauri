import type { SourceErrorInfo, SourceNode, SourcePath } from '../../../shared/api';

export interface NodeState {
  loading: boolean;
  error?: SourceErrorInfo;
  children?: SourceNode[];
  loadedAt?: number;
}

// `/` et `%` encodés dans les segments (chemins de fichiers du poste).
const encode = (segment: string) => segment.replaceAll('%', '%25').replaceAll('/', '%2F');
const decode = (segment: string) => segment.replaceAll('%2F', '/').replaceAll('%25', '%');

/** Clé d'un nœud : `connexion/cluster/namespace/pod`. */
export const nodeKey = (target: SourcePath) => [target.connectionId, ...target.path].map(encode).join('/');

export const parseKey = (key: string): SourcePath => {
  const [connectionId, ...path] = key.split('/').map(decode);
  return { connectionId, path };
};
