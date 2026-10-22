// Signaux « à la Angular » sur les runes de Svelte 5 : `s()` lit (et s'abonne), `s.set(v)` et
// `s.update(f)` écrivent. Les classes de session (flux, onglets, explorateur) restent ainsi du
// TypeScript ordinaire, testable sans composant, et réactif dans les vues Svelte.

import { untrack } from 'svelte';

export interface Signal<T> {
  (): T;
}

export interface WritableSignal<T> extends Signal<T> {
  set(value: T): void;
  update(fn: (value: T) => T): void;
  asReadonly(): Signal<T>;
}

export function signal<T>(initial: T): WritableSignal<T> {
  let value = $state.raw(initial);
  const read = (() => value) as WritableSignal<T>;
  read.set = (v: T) => {
    value = v;
  };
  read.update = (fn: (v: T) => T) => {
    value = fn(untrack(() => value));
  };
  read.asReadonly = () => () => value;
  return read;
}

export function computed<T>(fn: () => T): Signal<T> {
  const value = $derived.by(fn);
  return () => value;
}

/** Effet hors composant (services) ; renvoie la fonction d'arrêt. */
export function effect(fn: (onCleanup: (cleanup: () => void) => void) => void): () => void {
  return $effect.root(() => {
    $effect(() => {
      const cleanups: (() => void)[] = [];
      fn((c) => cleanups.push(c));
      return () => cleanups.forEach((c) => c());
    });
  });
}

export const untracked = untrack;
