/** Une valeur contient-elle `filter` (clé ou valeur, en profondeur) ? */
export function jsonMatches(key: string | number | null, value: unknown, filter: string): boolean {
  if (!filter) return true;
  if (key !== null && String(key).toLowerCase().includes(filter)) return true;
  if (value !== null && typeof value === 'object') {
    return Object.entries(value).some(([k, v]) => jsonMatches(k, v, filter));
  }
  return String(value).toLowerCase().includes(filter);
}
