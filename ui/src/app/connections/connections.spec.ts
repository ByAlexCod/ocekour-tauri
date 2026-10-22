import { defaultName, splitList } from './connections';

describe('fenêtre de connexion', () => {
  it('découpe les listes saisies', () => {
    expect(splitList(' a:9092, b:9092\nc;d ')).toEqual(['a:9092', 'b:9092', 'c', 'd']);
    expect(splitList('')).toEqual([]);
  });

  it('propose un nom', () => {
    const tls = { insecure: false };
    expect(defaultName({ type: 'rancher', name: '', url: 'https://rancher.intra/x', tls, clusters: [] })).toBe('rancher.intra');
    expect(defaultName({ type: 'kafka', name: '', brokers: ['k1.intra:9092'], tls, security: { protocol: 'PLAINTEXT', mechanism: 'PLAIN' } })).toBe('Kafka k1.intra');
    expect(defaultName({ type: 's3', name: '', url: 'pas une url', tls, pathStyle: true, accessKeyId: 'a', buckets: [] })).toBe('');
  });
});
