import { columnLabel, moveColumn, orderColumns } from './columns';

describe('ordre des colonnes', () => {
  const available = ['@time', '@level', 'traceId', 'status'];

  it('sans ordre enregistré : ordre par défaut', () => {
    expect(orderColumns(available, [])).toEqual(available);
  });

  it('ordre enregistré, colonnes inconnues ou en double ignorées', () => {
    expect(orderColumns(available, ['status', '@level', 'disparue', '@level', '@time', 'traceId'])).toEqual(['status', '@level', '@time', 'traceId']);
  });

  it('colonne absente de l’ordre (ajoutée depuis) : à la fin, dans l’ordre par défaut', () => {
    expect(orderColumns(available, ['@level', '@time'])).toEqual(['@level', '@time', 'traceId', 'status']);
    expect(orderColumns(available, ['traceId', '@time', '@level'])).toEqual(['traceId', '@time', '@level', 'status']);
  });

  it('déplacer une colonne', () => {
    expect(moveColumn(available, 'status', '@time')).toEqual(['status', '@time', '@level', 'traceId']);
    expect(moveColumn(available, '@time', 'traceId')).toEqual(['@level', 'traceId', '@time', 'status']);
    expect(moveColumn(available, '@time', '@time')).toEqual(available);
    expect(moveColumn(available, 'absente', '@time')).toEqual(available);
  });

  it('libellés', () => {
    expect(columnLabel('@time')).toBe('heure');
    expect(columnLabel('http.status')).toBe('http.status');
  });
});
