import { compileRules, fieldValue, newRule, ruleError, rulesAccept } from './log-filters';
import { LogBuffer } from './log-buffer';

const buffer = new LogBuffer(50);
buffer.push([
  { text: '{"level":"info","msg":"ok","path":"/health"}' },
  { text: '{"level":"error","msg":"boom","path":"/api/pay"}' },
  { text: 'texte simple sans json' },
]);
const [health, pay, plain] = buffer.entries;

describe('log-filters', () => {
  it('résout la valeur d’un champ, du message et de la ligne brute', () => {
    expect(fieldValue(health, 'path')).toBe('/health');
    expect(fieldValue(pay, 'path')).toBe('/api/pay');
    expect(fieldValue(plain, 'path')).toBeUndefined();
    expect(fieldValue(health, '@message')).toBe('ok');
    expect(fieldValue(plain, '@line')).toBe('texte simple sans json');
  });

  it('masque (liste noire) les lignes dont le champ correspond', () => {
    const rules = compileRules([newRule({ field: 'path', mode: 'exact', value: '/health' })]);
    expect(rulesAccept(health, rules)).toBe(false); // masquée
    expect(rulesAccept(pay, rules)).toBe(true);
    expect(rulesAccept(plain, rules)).toBe(true); // champ absent : pas masquée
  });

  it('regex sur un champ', () => {
    const rules = compileRules([newRule({ field: 'path', mode: 'regex', value: '^/api/' })]);
    expect(rulesAccept(pay, rules)).toBe(false);
    expect(rulesAccept(health, rules)).toBe(true);
  });

  it('garde seul (liste blanche) : ne garde que les correspondances', () => {
    const rules = compileRules([
      newRule({ field: 'path', mode: 'contains', value: 'api', action: 'only' }),
    ]);
    expect(rulesAccept(pay, rules)).toBe(true);
    expect(rulesAccept(health, rules)).toBe(false);
    expect(rulesAccept(plain, rules)).toBe(false); // champ absent : exclue par la liste blanche
  });

  it('la liste noire l’emporte sur la liste blanche', () => {
    const rules = compileRules([
      newRule({ field: '@line', mode: 'contains', value: 'level', action: 'only' }),
      newRule({ field: 'level', mode: 'exact', value: 'error', action: 'hide' }),
    ]);
    expect(rulesAccept(health, rules)).toBe(true);
    expect(rulesAccept(pay, rules)).toBe(false); // gardée par only, puis masquée par hide
  });

  it('ignore les règles désactivées, vides ou à regex invalide', () => {
    expect(compileRules([newRule({ enabled: false, value: 'x' })])).toHaveLength(0);
    expect(compileRules([newRule({ value: '' })])).toHaveLength(0);
    expect(compileRules([newRule({ mode: 'regex', value: '(' })])).toHaveLength(0);
    expect(ruleError(newRule({ mode: 'regex', value: '(' }))).toMatch(/invalide/);
    expect(ruleError(newRule({ mode: 'regex', value: '^ok$' }))).toBeUndefined();
  });
});
