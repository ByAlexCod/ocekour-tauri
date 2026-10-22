import { fileProfileName, profileKey, streamOptions, streamPath, targetKey, targetPath, targetTitle } from '../logs/target';
import { isPattern, localTarget } from './local-files.svelte';

describe('fichiers du poste', () => {
  it('reconnaît les motifs (jokers dans le nom seulement)', () => {
    expect(isPattern('C:\\logs\\app-*.log')).toBe(true);
    expect(isPattern('/var/log/app-?.log')).toBe(true);
    expect(isPattern('/var/*/app.log')).toBe(false);
    expect(isPattern('C:\\logs\\app.log')).toBe(false);
  });

  it('onglet d’un fichier, d’un motif, d’un fichier de motif', () => {
    const file = localTarget('C:\\logs\\app.log', { encoding: 'windows-1252' });
    expect(file).toEqual({ kind: 'file', path: ['C:\\logs\\app.log'], name: 'app.log', pattern: false, encoding: 'windows-1252' });
    expect(streamPath(file)).toEqual({ connectionId: 'local', path: ['C:\\logs\\app.log'] });
    expect(streamOptions(file, { follow: true })).toEqual({ follow: true, encoding: 'windows-1252' });
    expect(targetKey(file)).toBe('file/local/C:\\logs\\app.log');
    expect(targetTitle(file)).toEqual({ title: 'app.log', subtitle: 'fichier local' });

    const pattern = localTarget('/var/log/batch-*.log');
    expect(pattern.pattern).toBe(true);
    const one = localTarget('/var/log/batch-*.log', { file: 'batch-2026-10-22.log' });
    expect(one.pattern).toBe(false);
    expect(targetTitle(one).title).toBe('batch-2026-10-22.log');
    expect(targetPath(one)).toEqual(['/var/log/batch-*.log', 'batch-2026-10-22.log']);
    expect(targetKey(one)).not.toBe(targetKey(pattern));
  });

  it('clés de profil identiques à celles du cœur', () => {
    expect(fileProfileName('app.2026-10-22.log')).toBe('app.log');
    expect(fileProfileName('app-20261022.log')).toBe('app.log');
    expect(fileProfileName('app.log.3')).toBe('app.log');
    expect(fileProfileName('app-*.log')).toBe('app.log');
    expect(fileProfileName('app-1.log.gz')).toBe('app.log');
    expect(profileKey(localTarget('/l/app-*.log', { file: 'app-20261022.log' }))).toBe('fichier/app.log');
    expect(profileKey({ kind: 'file', connectionId: 's1', connectionName: 'srv', path: ['api'], name: 'API' })).toBe('sonde/srv/api');
  });
});
