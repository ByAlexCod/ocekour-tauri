import { KONAMI, KonamiDetector, SnakeGame } from './snake';

describe('code Konami', () => {
  it('détecte la suite, y compris après une fausse piste', () => {
    const detector = new KonamiDetector();
    const keys = ['ArrowUp', 'x', ...KONAMI.map((k) => (k.length === 1 ? k.toUpperCase() : k))];
    const hits = keys.map((k) => detector.push(k.replace(/^arrow/, 'Arrow')));
    expect(hits.filter(Boolean)).toHaveLength(1);
    expect(hits.at(-1)).toBe(true);
  });

  it('ArrowUp ArrowUp ArrowUp… repart correctement', () => {
    const detector = new KonamiDetector();
    const keys = ['arrowup', ...KONAMI];
    expect(keys.map((k) => detector.push(k)).at(-1)).toBe(true);
  });
});

describe('Snake', () => {
  it('avance, mange, grandit et accélère', () => {
    const game = new SnakeGame(() => 0);
    game.food = { x: 11, y: 10 };
    game.step();
    expect(game.score).toBe(1);
    expect(game.snake).toHaveLength(4);
    expect(game.snake[0]).toEqual({ x: 11, y: 10 });
    expect(game.delay).toBeLessThan(150);
  });

  it('demi-tour interdit ; perdu contre un mur', () => {
    const game = new SnakeGame(() => 0.99);
    game.turn({ x: -1, y: 0 });
    game.step();
    expect(game.snake[0]).toEqual({ x: 11, y: 10 });
    for (let i = 0; i < 20; i++) game.step();
    expect(game.over).toBe(true);
  });
});
