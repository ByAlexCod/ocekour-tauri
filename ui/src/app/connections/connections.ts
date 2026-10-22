import type { ConnectionDraft, ConnectionType } from '../../../shared/api';

export const CONNECTION_TYPES: { type: ConnectionType; label: string; hint: string }[] = [
  { type: 'rancher', label: 'Rancher', hint: 'Logs des conteneurs Kubernetes' },
  { type: 'kafka', label: 'Kafka', hint: 'Messages des topics (JSON, texte, Avro)' },
  { type: 's3', label: 'S3', hint: 'Explorateur de buckets' },
  { type: 'sonde', label: 'Sonde', hint: 'Logs exposés par une machine distante' },
];

export const SECRET_LABELS: Record<ConnectionType, string> = {
  rancher: 'Token API',
  kafka: 'Mot de passe',
  s3: 'Clé secrète',
  sonde: 'Jeton de la sonde',
};

/** Empreinte SHA-256 normalisée (`SHA256:AB:CD:…`), ou `null` si la saisie n'en est pas une. */
export function normalizeFingerprint(text: string): string | null {
  const hex = text.trim().replace(/^sha256:/i, '').replace(/[^0-9a-f]/gi, '');
  if (hex.length !== 64) return null;
  return 'SHA256:' + hex.toUpperCase().match(/../g)!.join(':');
}

/** Liste saisie avec des virgules, espaces ou retours à la ligne. */
export function splitList(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Nom proposé après un test réussi. */
export function defaultName(draft: ConnectionDraft): string {
  try {
    const host = draft.type === 'kafka' ? draft.brokers[0].split(':')[0] : new URL(draft.url).hostname;
    if (draft.type === 'rancher' || draft.type === 'sonde') return host;
    return `${draft.type === 'kafka' ? 'Kafka' : 'S3'} ${host}`;
  } catch {
    return '';
  }
}
