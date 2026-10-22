import type { Profile } from '../../../shared/api';
import type { FieldNames } from './parser';

/** Réglages effectifs d'un workload, après fusion des profils qui lui correspondent. */
export interface ResolvedProfile {
  /** Clé `cluster/namespace/workload` du workload affiché. */
  key: string;
  /** Profils appliqués, dans l'ordre (le premier prime). */
  matches: string[];
  fields: FieldNames;
  columns: string[];
  expand: string[];
  /** Colonnes dont le JSON s'affiche indenté dans la liste (`@message` : colonne message). */
  wrap: string[];
  parseNested: boolean;
}

/** Joker `*` : n'importe quel segment ; `**` : n'importe quoi, `/` compris. */
export function globToRegExp(glob: string): RegExp {
  const escaped = glob.replace(/[.+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(
    `^${escaped
      .replace(/\*\*/g, '\u0000')
      .replace(/\*/g, '[^/]*')
      .replace(/\u0000/g, '.*')}$`,
  );
}

/**
 * Fusionne tous les profils dont `match` correspond à la clé, dans l'ordre du fichier :
 * pour les champs clés et `parseNested`, le premier qui les définit l'emporte ; les
 * listes (`columns`, `expand`) sont cumulées. Le profil `*` sert donc de valeurs par défaut.
 */
export function resolveProfile(profiles: readonly Profile[], key: string): ResolvedProfile {
  const resolved: ResolvedProfile = {
    key,
    matches: [],
    fields: {},
    columns: [],
    expand: [],
    wrap: [],
    parseNested: false,
  };
  let parseNested: boolean | undefined;
  for (const profile of profiles) {
    if (profile.match !== '*' && !globToRegExp(profile.match).test(key)) continue;
    resolved.matches.push(profile.match);
    for (const role of ['time', 'level', 'message'] as const) {
      resolved.fields[role] ??= profile.fields[role];
    }
    for (const c of profile.columns) if (!resolved.columns.includes(c)) resolved.columns.push(c);
    for (const e of profile.expand) if (!resolved.expand.includes(e)) resolved.expand.push(e);
    for (const w of profile.wrap ?? []) if (!resolved.wrap.includes(w)) resolved.wrap.push(w);
    parseNested ??= profile.parseNested;
  }
  resolved.parseNested = parseNested ?? false;
  return resolved;
}
