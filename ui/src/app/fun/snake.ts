/** Suite ↑ ↑ ↓ ↓ ← → ← → B A. */
export const KONAMI = [
  'arrowup',
  'arrowup',
  'arrowdown',
  'arrowdown',
  'arrowleft',
  'arrowright',
  'arrowleft',
  'arrowright',
  'b',
  'a',
];

/** Détecte le code Konami : les 10 dernières touches forment la suite. */
export class KonamiDetector {
  private recent: string[] = [];

  /** `true` quand la dernière touche complète le code. */
  push(key: string): boolean {
    this.recent = [...this.recent, key.toLowerCase()].slice(-KONAMI.length);
    if (this.recent.length === KONAMI.length && this.recent.every((k, i) => k === KONAMI[i])) {
      this.recent = [];
      return true;
    }
    return false;
  }
}

export type Point = { x: number; y: number };

export const SIZE = 20;
export const CELL = 20;
export const BEST_KEY = 'ocekour.snake.best';
export const DIRECTIONS: Record<string, Point> = {
  arrowup: { x: 0, y: -1 },
  arrowdown: { x: 0, y: 1 },
  arrowleft: { x: -1, y: 0 },
  arrowright: { x: 1, y: 0 },
  z: { x: 0, y: -1 },
  w: { x: 0, y: -1 },
  s: { x: 0, y: 1 },
  q: { x: -1, y: 0 },
  a: { x: -1, y: 0 },
  d: { x: 1, y: 0 },
};

/** Logique du jeu, sans affichage (testable). */
export class SnakeGame {
  snake: Point[] = [
    { x: 10, y: 10 },
    { x: 9, y: 10 },
    { x: 8, y: 10 },
  ];
  direction: Point = { x: 1, y: 0 };
  private pending: Point[] = [];
  food: Point;
  score = 0;
  over = false;

  constructor(private readonly random: () => number = Math.random) {
    this.food = this.placeFood();
  }

  /** Changement de direction (demi-tour interdit) ; mis en file pour les touches rapides. */
  turn(next: Point) {
    const last = this.pending.at(-1) ?? this.direction;
    if (next.x === -last.x && next.y === -last.y) return;
    if (next.x === last.x && next.y === last.y) return;
    if (this.pending.length < 3) this.pending.push(next);
  }

  step() {
    if (this.over) return;
    this.direction = this.pending.shift() ?? this.direction;
    const head = { x: this.snake[0].x + this.direction.x, y: this.snake[0].y + this.direction.y };
    const eats = head.x === this.food.x && head.y === this.food.y;
    const body = eats ? this.snake : this.snake.slice(0, -1);
    if (
      head.x < 0 ||
      head.y < 0 ||
      head.x >= SIZE ||
      head.y >= SIZE ||
      body.some((p) => p.x === head.x && p.y === head.y)
    ) {
      this.over = true;
      return;
    }
    this.snake = [head, ...body];
    if (eats) {
      this.score++;
      this.food = this.placeFood();
    }
  }

  /** Délai entre deux pas : le jeu accélère avec le score. */
  get delay(): number {
    return Math.max(60, 150 - this.score * 4);
  }

  private placeFood(): Point {
    const free: Point[] = [];
    for (let y = 0; y < SIZE; y++) {
      for (let x = 0; x < SIZE; x++) {
        if (!this.snake.some((p) => p.x === x && p.y === y)) free.push({ x, y });
      }
    }
    return free[Math.floor(this.random() * free.length)] ?? { x: 0, y: 0 };
  }
}

export function readBest(): number {
  try {
    return Number(localStorage.getItem(BEST_KEY)) || 0;
  } catch {
    return 0;
  }
}
