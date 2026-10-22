/** Disposition verticale d'une liste virtuelle : position et hauteur de chaque ligne. */
export interface RowGeometry {
  readonly count: number;
  readonly total: number;
  offsetOf(index: number): number;
  heightOf(index: number): number;
  /** Ligne sous la position `y` (bornée à la liste). */
  indexAt(y: number): number;
}

/**
 * Hauteurs des lignes de la liste : toutes égales (cas courant, sans calcul), ou variables
 * (colonnes JSON indentées) avec sommes cumulées pour trouver une ligne par sa position.
 */
export class RowLayout implements RowGeometry {
  private readonly offsets?: Float64Array;

  constructor(
    readonly count: number,
    private readonly base: number,
    heights?: (index: number) => number,
  ) {
    if (heights) {
      this.offsets = new Float64Array(count + 1);
      for (let i = 0; i < count; i++) this.offsets[i + 1] = this.offsets[i] + heights(i);
    }
  }

  get total(): number {
    return this.offsets ? this.offsets[this.count] : this.count * this.base;
  }

  offsetOf(index: number): number {
    const i = Math.min(Math.max(index, 0), this.count);
    return this.offsets ? this.offsets[i] : i * this.base;
  }

  heightOf(index: number): number {
    return this.offsets ? this.offsets[index + 1] - this.offsets[index] : this.base;
  }

  /** Ligne sous la position `y` (bornée à la liste). */
  indexAt(y: number): number {
    if (this.count === 0) return 0;
    if (!this.offsets) return Math.min(Math.max(Math.floor(y / this.base), 0), this.count - 1);
    let low = 0;
    let high = this.count - 1;
    while (low < high) {
      const mid = (low + high + 1) >> 1;
      if (this.offsets[mid] <= y) low = mid;
      else high = mid - 1;
    }
    return low;
  }
}
