/** Largeur mémorisée sur le poste, sinon celle par défaut. */
export function storedWidth(key: string, fallback: number): number {
  try {
    return Number(localStorage.getItem(key)) || fallback;
  } catch {
    return fallback;
  }
}
