<script lang="ts">
  import { onMount } from 'svelte';
  import { BEST_KEY, CELL, DIRECTIONS, SIZE, SnakeGame, readBest } from './snake';

  /** Snake plein écran (code Konami). Échap : fermer ; Espace : pause ou rejouer. */
  let { onclose }: { onclose: () => void } = $props();

  const size = SIZE * CELL;
  let board: HTMLCanvasElement;
  let score = $state(0);
  let best = $state(readBest());
  let over = $state(false);
  let paused = $state(false);
  let game = new SnakeGame();
  let timer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    board.focus();
    tick();
    return () => clearTimeout(timer);
  });

  function onKey(event: KeyboardEvent) {
    const key = event.key.toLowerCase();
    event.preventDefault();
    event.stopPropagation();
    if (key === 'escape') return onclose();
    if (key === ' ') {
      if (game.over) restart();
      else paused = !paused;
      return;
    }
    const direction = DIRECTIONS[key];
    if (direction) game.turn(direction);
  }

  function restart() {
    game = new SnakeGame();
    score = 0;
    over = false;
    paused = false;
  }

  function tick() {
    if (!paused) {
      game.step();
      score = game.score;
      if (game.over && !over) {
        over = true;
        if (game.score > best) {
          best = game.score;
          try {
            localStorage.setItem(BEST_KEY, String(game.score));
          } catch {
            // record non mémorisé
          }
        }
      }
    }
    draw();
    timer = setTimeout(tick, game.delay);
  }

  function draw() {
    const ctx = board?.getContext('2d');
    if (!ctx) return;
    ctx.fillStyle = '#020617';
    ctx.fillRect(0, 0, size, size);
    ctx.fillStyle = '#0f172a';
    for (let i = 0; i < SIZE; i++) {
      for (let j = i % 2; j < SIZE; j += 2) ctx.fillRect(i * CELL, j * CELL, CELL, CELL);
    }
    ctx.fillStyle = '#f43f5e';
    ctx.beginPath();
    ctx.arc(game.food.x * CELL + CELL / 2, game.food.y * CELL + CELL / 2, CELL / 2 - 3, 0, Math.PI * 2);
    ctx.fill();
    game.snake.forEach((p, i) => {
      ctx.fillStyle = i === 0 ? '#6ee7b7' : game.over ? '#475569' : '#10b981';
      ctx.fillRect(p.x * CELL + 1, p.y * CELL + 1, CELL - 2, CELL - 2);
    });
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/80" role="dialog" aria-modal="true" aria-label="Snake" tabindex="-1" onclick={onclose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="rounded-xl border border-emerald-700 bg-slate-900 p-4 font-mono text-emerald-300 shadow-2xl" onclick={(e) => e.stopPropagation()}>
    <div class="mb-2 flex items-center gap-4 text-sm">
      <strong>🐍 Ocekour Snake</strong>
      <span>score {score}</span>
      <span class="text-emerald-600">record {best}</span>
      <span class="flex-1"></span>
      <button type="button" class="text-emerald-600 hover:text-emerald-300" onclick={onclose}>Échap ✕</button>
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <canvas bind:this={board} tabindex="0" class="block rounded outline-none" width={size} height={size} onkeydown={onKey}></canvas>
    <p class="mt-2 text-center text-xs text-emerald-600">
      {#if over}Perdu ! Espace pour rejouer.{:else if paused}Pause · Espace pour reprendre{:else}Flèches (ou ZQSD / WASD) · Espace : pause{/if}
    </p>
  </div>
</div>
