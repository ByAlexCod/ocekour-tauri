// Recherche dans les lignes : texte (sensible ou non à la casse) ou expression régulière.

export interface SearchOptions {
  text: string;
  caseSensitive: boolean;
  regex: boolean;
}

export interface Matcher {
  test(text: string): boolean;
  /** Plages [début, fin[ des correspondances, pour le surlignage. */
  ranges(text: string): [number, number][];
}

export type Segment = { text: string; match: boolean };

const escapeRegExp = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * Construit le matcher d'une recherche ; `undefined` si la recherche est vide.
 * Lève une erreur (message lisible) si l'expression régulière est invalide.
 */
export function buildMatcher(options: SearchOptions): Matcher | undefined {
  if (!options.text) return undefined;
  const source = options.regex ? options.text : escapeRegExp(options.text);
  let re: RegExp;
  try {
    re = new RegExp(source, options.caseSensitive ? 'g' : 'gi');
  } catch (e) {
    throw new Error(`Expression régulière invalide : ${(e as Error).message}`);
  }
  const single = new RegExp(re.source, re.flags.replace('g', ''));
  return {
    test: (text) => single.test(text),
    ranges: (text) => {
      const ranges: [number, number][] = [];
      re.lastIndex = 0;
      for (let m = re.exec(text); m; m = re.exec(text)) {
        if (m[0].length === 0) {
          re.lastIndex++; // regex qui correspond à la chaîne vide : on avance
          continue;
        }
        ranges.push([m.index, m.index + m[0].length]);
        if (ranges.length > 200) break;
      }
      return ranges;
    },
  };
}

/** Découpe un texte en morceaux surlignés ou non. */
export function segments(text: string, matcher: Matcher | undefined): Segment[] {
  if (!matcher) return [{ text, match: false }];
  const result: Segment[] = [];
  let last = 0;
  for (const [start, end] of matcher.ranges(text)) {
    if (start > last) result.push({ text: text.slice(last, start), match: false });
    result.push({ text: text.slice(start, end), match: true });
    last = end;
  }
  if (last < text.length) result.push({ text: text.slice(last), match: false });
  return result.length ? result : [{ text, match: false }];
}
