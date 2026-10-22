import { flushSync } from 'svelte';
import { computed, effect, signal } from './signals.svelte';

describe('signaux', () => {
  it('lecture, écriture, dérivé et effet', () => {
    const a = signal(1);
    const double = computed(() => a() * 2);
    const seen: number[] = [];
    const stop = effect(() => {
      seen.push(double());
    });
    flushSync();
    a.set(2);
    flushSync();
    a.update((v) => v + 1);
    flushSync();
    expect(double()).toBe(6);
    expect(seen).toEqual([2, 4, 6]);
    stop();
    a.set(10);
    flushSync();
    expect(seen).toEqual([2, 4, 6]);
  });
});
