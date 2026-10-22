import { LogBuffer } from './log-buffer';

describe('LogBuffer', () => {
  it('analyse à l’arrivée et retire les plus anciennes au-delà de la capacité', () => {
    const buffer = new LogBuffer(3);
    buffer.push(['a', 'b', 'c', 'd'].map((text) => ({ text: `{"level":"info","msg":"${text}"}` })));
    buffer.notice('Conteneur redémarré');
    expect(buffer.entries.map((e) => e.parsed.message)).toEqual(['c', 'd', 'Conteneur redémarré']);
    expect(buffer.entries.map((e) => e.seq)).toEqual([2, 3, 4]);
    expect(buffer.dropped).toBe(2);
    expect(buffer.total).toBe(5);
  });

  it('regroupe les notices consécutives identiques (compteur + occurrences)', () => {
    const buffer = new LogBuffer(100);
    buffer.push([{ text: '{"level":"info","msg":"a"}' }]);
    const first = buffer.notice('Flux interrompu');
    expect(first.created).toBe(true);
    const second = buffer.notice('Flux interrompu');
    expect(second.created).toBe(false);
    expect(second.entry).toBe(first.entry);
    buffer.notice('Flux interrompu');
    // Une seule ligne de notice, avec le compteur à 3 et 3 horodatages.
    const notices = buffer.entries.filter((e) => e.notice);
    expect(notices.length).toBe(1);
    expect(notices[0].repeat).toBe(3);
    expect(notices[0].times?.length).toBe(3);
    // Une notice de texte différent n'est pas regroupée.
    const other = buffer.notice('Conteneur redémarré');
    expect(other.created).toBe(true);
    expect(buffer.entries.filter((e) => e.notice).length).toBe(2);
    // Une notice identique mais non consécutive (ligne de log entre les deux) est distincte.
    buffer.push([{ text: '{"level":"info","msg":"b"}' }]);
    const again = buffer.notice('Conteneur redémarré');
    expect(again.created).toBe(true);
    expect(buffer.entries.filter((e) => e.notice).length).toBe(3);
  });

  it('ré-analyse les lignes quand les champs du profil changent', () => {
    const buffer = new LogBuffer(10);
    buffer.push([{ text: '{"txt":"bonjour","msg":"autre"}' }]);
    expect(buffer.entries[0].parsed.message).toBe('autre');
    buffer.setFields({ message: 'txt' });
    expect(buffer.entries[0].parsed.message).toBe('bonjour');
  });

  it('message Kafka : la recherche porte aussi sur la clé et les headers', () => {
    const buffer = new LogBuffer(10);
    buffer.push([
      {
        text: '{"msg":"payé"}',
        meta: {
          partition: 2,
          offset: '41',
          key: 'cmd-7',
          headers: { trace: 'abc' },
          format: 'json',
        },
      },
    ]);
    const [entry] = buffer.entries;
    expect(entry.meta?.offset).toBe('41');
    expect(entry.haystack).toContain('cmd-7');
    expect(entry.haystack).toContain('trace=abc');
  });
});
