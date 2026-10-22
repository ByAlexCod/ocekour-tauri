import { buildMatcher } from './filter';
import type { LogEntry } from './log-buffer';
import { getFieldDeep } from './parser';

// Règles de filtrage d'un onglet : masquer (liste noire) ou garder seul (liste blanche) des
// lignes selon la valeur d'un champ (chemin JSON, `@message` ou `@line`), en exact / contient / regex.

export type RuleMode = 'exact' | 'contains' | 'regex';
export type RuleAction = 'hide' | 'only';

export interface LogFilterRule {
  id: string;
  enabled: boolean;
  /** `@line` (ligne brute), `@message` (message analysé), ou un chemin de champ JSON (`http.path`). */
  field: string;
  mode: RuleMode;
  value: string;
  caseSensitive: boolean;
  action: RuleAction;
}

/** Règle compilée : prédicat prêt à l'emploi. */
export interface CompiledRule {
  action: RuleAction;
  test: (entry: LogEntry) => boolean;
}

/** Valeur d'un champ d'une ligne, en texte (ou `undefined` si absent). */
export function fieldValue(entry: LogEntry, field: string): string | undefined {
  if (field === '@line') return entry.text;
  if (field === '@message' || field === '') return entry.parsed.message;
  const json = entry.parsed.json;
  if (!json) return undefined;
  const value = getFieldDeep(json, field);
  if (value === undefined || value === null) return undefined;
  return typeof value === 'string' ? value : typeof value === 'object' ? JSON.stringify(value) : String(value);
}

/** Message d'erreur d'une règle (regex invalide), sinon `undefined`. */
export function ruleError(rule: LogFilterRule): string | undefined {
  if (rule.mode !== 'regex' || !rule.value) return undefined;
  try {
    new RegExp(rule.value);
    return undefined;
  } catch (e) {
    return `Expression régulière invalide : ${(e as Error).message}`;
  }
}

/** Compile les règles actives et valides en prédicats (les invalides sont ignorées). */
export function compileRules(rules: readonly LogFilterRule[]): CompiledRule[] {
  const compiled: CompiledRule[] = [];
  for (const rule of rules) {
    if (!rule.enabled || !rule.value) continue;
    let test: (entry: LogEntry) => boolean;
    if (rule.mode === 'regex') {
      let matcher;
      try {
        matcher = buildMatcher({ text: rule.value, regex: true, caseSensitive: rule.caseSensitive });
      } catch {
        continue; // regex invalide : règle ignorée
      }
      if (!matcher) continue;
      const m = matcher;
      test = (entry) => {
        const v = fieldValue(entry, rule.field);
        return v !== undefined && m.test(v);
      };
    } else {
      const needle = rule.caseSensitive ? rule.value : rule.value.toLowerCase();
      const exact = rule.mode === 'exact';
      test = (entry) => {
        let v = fieldValue(entry, rule.field);
        if (v === undefined) return false;
        if (!rule.caseSensitive) v = v.toLowerCase();
        return exact ? v === needle : v.includes(needle);
      };
    }
    compiled.push({ action: rule.action, test });
  }
  return compiled;
}

/**
 * Une ligne passe-t-elle les règles ? Liste blanche (`only`) d'abord (OU entre elles), puis liste
 * noire (`hide`, OU entre elles) qui l'emporte. Sans règle : tout passe.
 */
export function rulesAccept(entry: LogEntry, compiled: readonly CompiledRule[]): boolean {
  if (!compiled.length) return true;
  let hasOnly = false;
  let keptByOnly = false;
  for (const rule of compiled) {
    if (rule.action === 'only') {
      hasOnly = true;
      if (rule.test(entry)) keptByOnly = true;
    }
  }
  if (hasOnly && !keptByOnly) return false;
  for (const rule of compiled) {
    if (rule.action === 'hide' && rule.test(entry)) return false;
  }
  return true;
}

/** Nouvelle règle vide (valeurs par défaut). */
export function newRule(patch: Partial<LogFilterRule> = {}): LogFilterRule {
  return {
    id: Math.random().toString(36).slice(2, 10),
    enabled: true,
    field: 'path',
    mode: 'contains',
    value: '',
    caseSensitive: false,
    action: 'hide',
    ...patch,
  };
}
