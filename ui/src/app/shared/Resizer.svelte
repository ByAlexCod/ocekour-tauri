<script lang="ts">
  /**
   * Séparateur vertical déplaçable : glisser pour changer une largeur, double-clic pour la
   * largeur par défaut, flèches gauche / droite au clavier. La largeur est mémorisée sur le
   * poste (`storageKey`).
   */
  let {
    width = $bindable(),
    label,
    storageKey,
    defaultWidth,
    min = 200,
    maxRatio = 0.6,
    direction = 1,
  }: {
    width: number;
    label: string;
    storageKey: string;
    defaultWidth: number;
    min?: number;
    /** Part maximale de la fenêtre. */
    maxRatio?: number;
    /** 1 : le panneau est à gauche du séparateur ; -1 : à droite. */
    direction?: 1 | -1;
  } = $props();

  function start(event: PointerEvent) {
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const initial = width;
    const move = (e: PointerEvent) => apply(initial + (e.clientX - event.clientX) * direction, false);
    const up = () => {
      handle.removeEventListener('pointermove', move);
      save();
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up, { once: true });
  }

  function apply(next: number, persist: boolean) {
    width = Math.round(Math.min(Math.max(next, min), window.innerWidth * maxRatio));
    if (persist) save();
  }

  function save() {
    try {
      localStorage.setItem(storageKey, String(width));
    } catch {
      // stockage indisponible : largeur non mémorisée
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft') apply(width - 20 * direction, true);
    else if (e.key === 'ArrowRight') apply(width + 20 * direction, true);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  role="separator"
  tabindex="0"
  aria-orientation="vertical"
  aria-label={label}
  aria-valuenow={width}
  aria-valuemin={min}
  title="Glisser pour redimensionner · double-clic : largeur par défaut"
  class="w-1 shrink-0 cursor-col-resize bg-slate-200 hover:bg-sky-400 focus-visible:bg-sky-500 focus-visible:outline-none dark:bg-slate-800"
  onpointerdown={start}
  ondblclick={() => apply(defaultWidth, true)}
  {onkeydown}
></div>
