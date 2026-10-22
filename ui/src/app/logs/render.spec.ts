import { RowLayout } from './row-layout';
import { clip, esc, visibleRange } from './render';

describe('rendu des lignes', () => {
  it('échappe le HTML', () => {
    expect(esc(`<a href="x">'&'</a>`)).toBe('&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;');
  });

  it('borne le texte et rend les retours à la ligne visibles', () => {
    expect(clip('a\nb')).toBe('a ↵ b');
    expect(clip('x'.repeat(500))).toHaveLength(401);
  });

  it('ne rend que la zone visible et une marge', () => {
    const layout = new RowLayout(10_000, 20);
    expect(visibleRange(layout, 0, 400)).toEqual({ start: 0, end: 51 });
    expect(visibleRange(layout, 20_000, 400)).toEqual({ start: 970, end: 1051 });
    expect(visibleRange(new RowLayout(0, 20), 0, 400)).toEqual({ start: 0, end: 0 });
  });
});
