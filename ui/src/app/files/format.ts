/** Taille lisible (o, Ko, Mo, Go). */
export function size(bytes: number | undefined): string {
  if (bytes === undefined) return '';
  const units = ['o', 'Ko', 'Mo', 'Go', 'To'];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${i === 0 ? n : n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`.replace('.', ',');
}

/** Ancienneté lisible d'une date ISO (« 5 min », « 3 h », « 2 j »). */
export function age(iso: string | undefined, now = Date.now()): string {
  if (!iso) return '';
  const minutes = Math.floor((now - Date.parse(iso)) / 60_000);
  if (minutes < 60) return `${Math.max(minutes, 0)} min`;
  if (minutes < 60 * 48) return `${Math.floor(minutes / 60)} h`;
  return `${Math.floor(minutes / 1440)} j`;
}
