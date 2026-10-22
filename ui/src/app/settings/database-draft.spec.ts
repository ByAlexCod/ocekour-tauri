import { canSubmit, type DatabaseForm, defaultPort, engineLabel, toDraft } from './database-draft';

const form = (patch: Partial<DatabaseForm> = {}): DatabaseForm => ({
  engine: 'postgres',
  name: ' Prod ',
  host: ' db.ex ',
  port: '',
  database: '',
  user: ' lecteur ',
  password: '',
  ssl: false,
  oracleSid: false,
  insecure: false,
  ...patch,
});

describe('formulaire de base de données', () => {
  it('libellés et ports par défaut', () => {
    expect(engineLabel('sqlserver')).toBe('SQL Server');
    expect(defaultPort('oracle')).toBe('1521');
    expect(defaultPort('postgres')).toBe('5432');
  });

  it('nom, hôte et utilisateur obligatoires', () => {
    expect(canSubmit(form())).toBe(true);
    expect(canSubmit(form({ host: '  ' }))).toBe(false);
    expect(canSubmit(form({ user: '' }))).toBe(false);
  });

  it('brouillon nettoyé', () => {
    expect(toDraft(form())).toEqual({
      id: undefined,
      engine: 'postgres',
      name: 'Prod',
      host: 'db.ex',
      port: undefined,
      database: undefined,
      user: 'lecteur',
      ssl: false,
      oracleSid: false,
      tls: { insecure: false },
      password: undefined,
    });
  });

  it('port, base, mot de passe et CA', () => {
    const d = toDraft(form({ port: '15432', database: ' app ', password: 's', ssl: true, caFile: 'C:/ca.cer' }));
    expect(d).toMatchObject({ port: 15432, database: 'app', password: 's', ssl: true, tls: { insecure: false, caFile: 'C:/ca.cer' } });
    expect(toDraft(form({ port: 'abc' })).port).toBeUndefined();
    expect(toDraft(form({ port: '-3' })).port).toBeUndefined();
  });
});
