import type { Connection } from '../../../shared/api';
import { ConnectionSchema } from '../../../shared/config';
import type { TabState } from '../logs/tabs.service';
import { decodeLink, encodeLink, findConnection, findLink, linkFor, stateFor } from './share-link';

const rancher = ConnectionSchema.parse({
  id: 'rancher-1',
  type: 'rancher',
  name: 'Prod',
  url: 'https://rancher.intra',
  clusters: [{ id: 'c-1', name: 'prod' }],
}) as Connection;
const kafka = ConnectionSchema.parse({
  id: 'kafka-1',
  type: 'kafka',
  name: 'Kafka',
  brokers: ['k1:9092', 'k2:9092'],
  security: { username: 'moi' },
}) as Connection;
const s3 = ConnectionSchema.parse({
  id: 's3-1',
  type: 's3',
  name: 'S3',
  url: 'https://s3.intra',
  accessKeyId: 'AKIA-SECRETE',
}) as Connection;

const containerState: TabState = {
  target: {
    kind: 'container',
    connectionId: 'rancher-1',
    clusterId: 'c-1',
    clusterName: 'prod',
    namespace: 'app',
    pod: 'api-7d9f-x2k4p',
    container: 'api',
    workload: 'api',
    node: { id: 'api', kind: 'container', expandable: false, hasPrevious: true },
  },
  options: { follow: true, tailLines: 1000 },
  search: { text: 'paiement é', caseSensitive: false, regex: false },
  levels: ['warn', 'error'],
  searchMode: 'highlight',
};

describe('lien de partage', () => {
  it('aller-retour : même vue, connexion retrouvée par URL (casse, / final)', () => {
    const text = encodeLink(linkFor(containerState, rancher));
    expect(text).toMatch(/^ocekour:\/\/open\?d=[A-Za-z0-9_-]+$/);
    const link = decodeLink(`Regarde ça : ${text} merci`);
    const other = { ...rancher, id: 'autre-id', url: 'https://RANCHER.intra/' } as Connection;
    const connection = findConnection(link, [kafka, other])!;
    expect(connection.id).toBe('autre-id');
    const state = stateFor(link, connection);
    expect(state.target).toEqual({
      ...containerState.target,
      connectionId: 'autre-id',
      // L'état du conteneur (redémarrages…) n'est pas transmis : relu à l'ouverture.
      node: { id: 'api', kind: 'container', expandable: false },
    });
    expect(state).toMatchObject({
      options: containerState.options,
      search: containerState.search,
      levels: ['warn', 'error'],
      searchMode: 'highlight',
    });
  });

  it('jamais de secret ni d’identifiant local dans le lien', () => {
    const payload = (link: string) =>
      atob(link.split('d=')[1].replace(/-/g, '+').replace(/_/g, '/'));
    const s3Link = encodeLink(
      linkFor(
        {
          target: { kind: 'bucket', connectionId: 's3-1', connectionName: 'S3', bucket: 'b' },
          prefix: 'logs/',
        },
        s3,
      ),
    );
    expect(payload(s3Link)).not.toContain('AKIA');
    expect(payload(s3Link)).not.toContain('s3-1');
    const kafkaLink = encodeLink(
      linkFor(
        {
          target: { kind: 'topic', connectionId: 'kafka-1', connectionName: 'Kafka', topic: 't' },
          options: { follow: false },
        },
        kafka,
      ),
    );
    expect(payload(kafkaLink)).not.toContain('moi');
  });

  it('Kafka : connexion retrouvée par un broker en commun', () => {
    const link = decodeLink(
      encodeLink({ v: 1, t: 'kafka', brokers: ['K2:9092'], topic: 't', options: { follow: true } }),
    );
    expect(findConnection(link, [rancher, kafka])?.id).toBe('kafka-1');
    expect(findConnection(link, [rancher])).toBeUndefined();
  });

  it('liens invalides : messages lisibles', () => {
    expect(() => decodeLink('bonjour')).toThrow(/pas un lien Ocekour/);
    expect(() => decodeLink('ocekour://open?d=@@')).toThrow(/pas un lien/);
    expect(() => decodeLink('ocekour://open?d=abc')).toThrow(/illisible/);
    const future = btoa(JSON.stringify({ v: 2 })).replace(/=+$/, '');
    expect(() => decodeLink(`ocekour://open?d=${future}`)).toThrow(/autre version/);
    const partial = btoa(JSON.stringify({ v: 1, t: 's3' })).replace(/=+$/, '');
    expect(() => decodeLink(`ocekour://open?d=${partial}`)).toThrow(/incomplet/);
    expect(findLink('rien')).toBeUndefined();
  });
});

describe('liens de partage — sonde', () => {
  const sonde = ConnectionSchema.parse({
    id: 'sonde-1',
    type: 'sonde',
    name: 'srv-recette',
    url: 'https://srv-recette:47120/',
    tls: { insecure: false, fingerprint: 'SHA256:AB' },
  }) as Connection;

  it('aller-retour d’un fichier de motif, sans jeton ni empreinte', () => {
    const state: TabState = {
      target: { kind: 'file', connectionId: 'sonde-1', connectionName: 'srv-recette', path: ['batch', 'batch-2.log'], name: 'batch' },
      options: { follow: true, tailLines: 500 },
      search: { text: 'ERROR', caseSensitive: false, regex: false },
    };
    const text = encodeLink(linkFor(state, sonde));
    const link = decodeLink(`voici : ${text} merci`);
    expect(link).toMatchObject({ t: 'sonde', url: 'https://srv-recette:47120/', source: 'batch', file: 'batch-2.log' });
    expect(text).not.toContain('SHA256');
    const other = { ...sonde, id: 'sonde-2' } as Connection;
    expect(findConnection(link, [rancher, other])?.id).toBe('sonde-2');
    expect(stateFor(link, other)).toMatchObject({
      target: { kind: 'file', connectionId: 'sonde-2', path: ['batch', 'batch-2.log'] },
      options: { follow: true, tailLines: 500 },
      search: { text: 'ERROR' },
    });
    expect(findLink(text)).toBe(text);
  });

  it('source entière (sans fichier)', () => {
    const link = linkFor({ target: { kind: 'file', connectionId: 'sonde-1', path: ['api'], name: 'API' } }, sonde);
    expect(link).toEqual({ v: 1, t: 'sonde', url: 'https://srv-recette:47120/', source: 'api', options: { follow: true }, filters: undefined });
    expect(stateFor(decodeLink(encodeLink(link)), sonde).target).toMatchObject({ path: ['api'] });
  });
});
