import { accountDirty, HELP, PLACEHOLDER, WRITE_HINT } from './services-help';

describe('comptes de services', () => {
  it('aide, exemple d’URL et portée d’écriture pour chaque service', () => {
    for (const id of ['gitlab', 'jira', 'confluence', 'rocketchat'] as const) {
      expect(HELP[id].steps.length).toBeGreaterThan(0);
      expect(PLACEHOLDER[id]).toMatch(/^https:\/\//);
      expect(WRITE_HINT[id]).toBeTruthy();
    }
  });

  it('formulaire modifié : URL (sans / final), TLS, CA', () => {
    const account = { url: 'https://gitlab.ex', tls: { insecure: false } };
    expect(accountDirty(account, { url: ' https://gitlab.ex// ', insecure: false })).toBe(false);
    expect(accountDirty(account, { url: 'https://autre.ex', insecure: false })).toBe(true);
    expect(accountDirty(account, { url: 'https://gitlab.ex', insecure: true })).toBe(true);
    expect(accountDirty(account, { url: 'https://gitlab.ex', insecure: false, caFile: 'C:/ca.cer' })).toBe(true);
    expect(accountDirty({ tls: { insecure: false } }, { url: '', insecure: false })).toBe(false);
  });
});
