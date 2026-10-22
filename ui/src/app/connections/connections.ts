import type { ConnectionDraft, ConnectionType } from '../../../shared/api';

export const CONNECTION_TYPES: { type: ConnectionType; label: string; hint: string }[] = [
  { type: 'rancher', label: 'Rancher', hint: 'Logs des conteneurs Kubernetes' },
  { type: 'kafka', label: 'Kafka', hint: 'Messages des topics (JSON, texte, Avro)' },
  { type: 's3', label: 'S3', hint: 'Explorateur de buckets' },
];

export const SECRET_LABELS: Record<ConnectionType, string> = {
  rancher: 'Token API',
  kafka: 'Mot de passe',
  s3: 'Clé secrète',
};

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
    return draft.type === 'rancher' ? host : `${draft.type === 'kafka' ? 'Kafka' : 'S3'} ${host}`;
  } catch {
    return '';
  }
}
