// Colonnes de la liste de logs : ordre choisi et colonnes masquées (profil : `order`, `hidden`).
// Le message reste toujours la dernière colonne (il prend la place restante).

/** Colonnes fixes (hors champs du profil) et leur libellé. */
export const COLUMN_LABELS: Readonly<Record<string, string>> = {
  '@time': 'heure',
  '@source': 'source',
  '@level': 'niveau',
  '@part': 'partition',
  '@offset': 'offset',
  '@key': 'clé',
};

export const columnLabel = (column: string) => COLUMN_LABELS[column] ?? column;

/**
 * Colonnes disponibles dans l'ordre enregistré : celles de `order` d'abord (inconnues ou en
 * double ignorées), puis celles qui n'y sont pas (ajoutées depuis), dans leur ordre par défaut.
 */
export function orderColumns(available: readonly string[], order: readonly string[]): string[] {
  const result = order.filter((c, i) => available.includes(c) && order.indexOf(c) === i);
  return [...result, ...available.filter((c) => !result.includes(c))];
}

/** Déplace `column` à la place de `target` (avant elle si on remonte, après si on descend). */
export function moveColumn(columns: readonly string[], column: string, target: string): string[] {
  const from = columns.indexOf(column);
  const to = columns.indexOf(target);
  if (from < 0 || to < 0 || from === to) return [...columns];
  const next = [...columns];
  next.splice(from, 1);
  next.splice(to, 0, column);
  return next;
}
