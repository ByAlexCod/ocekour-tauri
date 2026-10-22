import { RowLayout } from './row-layout';

describe('RowLayout', () => {
  it('hauteurs égales : calcul direct', () => {
    const layout = new RowLayout(10, 20);
    expect(layout.total).toBe(200);
    expect(layout.offsetOf(3)).toBe(60);
    expect(layout.indexAt(59)).toBe(2);
    expect(layout.indexAt(-5)).toBe(0);
    expect(layout.indexAt(10_000)).toBe(9);
  });

  it('hauteurs variables : sommes cumulées et recherche par position', () => {
    const heights = [20, 100, 20, 52];
    const layout = new RowLayout(4, 20, (i) => heights[i]);
    expect(layout.total).toBe(192);
    expect([0, 1, 2, 3].map((i) => layout.offsetOf(i))).toEqual([0, 20, 120, 140]);
    expect(layout.heightOf(1)).toBe(100);
    expect(layout.indexAt(0)).toBe(0);
    expect(layout.indexAt(20)).toBe(1);
    expect(layout.indexAt(119)).toBe(1);
    expect(layout.indexAt(120)).toBe(2);
    expect(layout.indexAt(191)).toBe(3);
    expect(new RowLayout(0, 20, () => 20).indexAt(5)).toBe(0);
  });
});
