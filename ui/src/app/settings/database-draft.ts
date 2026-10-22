import type { DatabaseDraft, DatabaseEngine } from '../../../shared/api';

export const ENGINES: { value: DatabaseEngine; label: string; port: number }[] = [
  { value: 'postgres', label: 'PostgreSQL', port: 5432 },
  { value: 'sqlserver', label: 'SQL Server', port: 1433 },
  { value: 'oracle', label: 'Oracle', port: 1521 },
];

export const engineLabel = (e: DatabaseEngine) => ENGINES.find((x) => x.value === e)?.label ?? e;
export const defaultPort = (e: DatabaseEngine) => String(ENGINES.find((x) => x.value === e)?.port ?? '');

/** Saisie du formulaire « Base de données ». */
export interface DatabaseForm {
  id?: string;
  engine: DatabaseEngine;
  name: string;
  host: string;
  port: string;
  database: string;
  user: string;
  password: string;
  ssl: boolean;
  oracleSid: boolean;
  insecure: boolean;
  caFile?: string;
}

export const canSubmit = (f: DatabaseForm) => !!f.name.trim() && !!f.host.trim() && !!f.user.trim();

/** Brouillon envoyé au cœur : champs nettoyés, port facultatif, mot de passe inchangé si vide. */
export function toDraft(f: DatabaseForm): DatabaseDraft {
  const port = Number(f.port);
  return {
    id: f.id,
    engine: f.engine,
    name: f.name.trim(),
    host: f.host.trim(),
    port: f.port.trim() && Number.isInteger(port) && port > 0 ? port : undefined,
    database: f.database.trim() || undefined,
    user: f.user.trim(),
    ssl: f.ssl,
    oracleSid: f.oracleSid,
    tls: { insecure: f.insecure, ...(f.caFile ? { caFile: f.caFile } : {}) },
    password: f.password || undefined,
  };
}
