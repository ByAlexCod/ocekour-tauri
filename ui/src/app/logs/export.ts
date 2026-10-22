import type { LogEntry } from './log-buffer';

export type ExportFormat = 'ndjson' | 'text';

/**
 * Une ligne exportée. NDJSON : la ligne JSON d'origine telle quelle ; sinon un objet
 * `{time, message}` ; message Kafka : horodatage, partition, offset, clé, headers, valeur.
 * Texte : la ligne brute (préfixée de la position Kafka pour un topic).
 */
export function exportLine(entry: LogEntry, format: ExportFormat): string {
  const meta = entry.meta;
  const src = entry.source?.label;
  if (format === 'text') {
    const prefix = src ? `[${src}] ` : '';
    if (!meta) return prefix + entry.text;
    const key = meta.key !== undefined ? ` ${meta.key}` : '';
    return `${prefix}${entry.ts ?? ''} p${meta.partition}@${meta.offset}${key} ${entry.text}`;
  }
  // Assemblage (plusieurs sources) : objet structuré avec la source, pour rester exploitable.
  if (src) {
    return JSON.stringify(
      meta
        ? {
            source: src,
            timestamp: entry.ts,
            partition: meta.partition,
            offset: meta.offset,
            key: meta.key ?? null,
            headers: meta.headers ?? {},
            value: entry.parsed.json ?? entry.text,
          }
        : { source: src, time: entry.ts, log: entry.parsed.json ?? entry.text },
    );
  }
  if (meta) {
    return JSON.stringify({
      timestamp: entry.ts,
      partition: meta.partition,
      offset: meta.offset,
      key: meta.key ?? null,
      headers: meta.headers ?? {},
      value: entry.parsed.json ?? entry.text,
    });
  }
  if (entry.parsed.json) return entry.text.trim();
  return JSON.stringify({ time: entry.ts, message: entry.text });
}

/** Contenu du fichier : une ligne par entrée (les événements d'Ocekour sont exclus). */
export function exportContent(entries: readonly LogEntry[], format: ExportFormat): string {
  const lines = entries.filter((e) => !e.notice).map((e) => exportLine(e, format));
  return lines.length ? lines.join('\n') + '\n' : '';
}

/** Nom de fichier proposé : cible + date locale, sans caractère interdit sous Windows. */
export function exportFileName(name: string, now = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
  return `${name.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_')}-${stamp}`;
}
