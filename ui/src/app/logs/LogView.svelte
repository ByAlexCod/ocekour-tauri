<script lang="ts">
  import { onDestroy, onMount, tick, untrack } from 'svelte';
  import type { LogDownloadSource } from '../../../shared/api';
  import { analytics } from '../core/analytics';
  import { configService } from '../core/config.svelte';
  import { notifications } from '../core/notification.svelte';
  import { shareService as share } from '../share/share.service';
  import { AssemblySession } from './assembly-session';
  import LogDetail from './detail/LogDetail.svelte';
  import { exportContent, exportFileName } from './export';
  import { FieldActions, provideFieldActions } from './field-actions';
  import { buildMatcher, type Matcher, type SearchOptions, segments } from './filter';
  import { MESSAGE_COLUMN, wrappedValue } from './json-cell';
  import type { LogEntry } from './log-buffer';
  import { compileRules, type LogFilterRule, newRule, ruleError } from './log-filters';
  import { ROW_HEIGHT, type LogSession } from './log-session';
  import { type Level, LEVELS } from './parser';
  import { resolveProfile } from './profiles';
  import { LEVEL_CLASS, baseClass, clip, columnText, esc, formatTime, fullDate, noticeTime, visibleRange } from './render';
  import { profileKey, type StreamTarget, streamPath, targetPath } from './target';

  /** Affichage d'un flux : barre d'options, état, liste virtualisée à défilement automatique. */
  let {
    session,
    onswitchpod,
    oneditconfig,
  }: {
    session: LogSession | AssemblySession;
    /** Pod supprimé : l'utilisateur choisit un pod de remplacement. */
    onswitchpod: (pod: string) => void;
    /** Ouvrir config.yaml (profil du workload). */
    oneditconfig: () => void;
  } = $props();

  /** Délai avant d'appliquer une recherche tapée (évite un filtrage complet par touche). */
  const SEARCH_DEBOUNCE_MS = 150;
  /** Ligne de texte d'une cellule JSON indentée (text-xs), et marge verticale de la ligne. */
  const LINE_HEIGHT = 16;
  const WRAP_PADDING = 4;
  const TAIL_CHOICES = [100, 1_000, 10_000];
  const SINCE_CHOICES = [
    { label: 'Tout', seconds: undefined },
    { label: '5 min', seconds: 300 },
    { label: '1 h', seconds: 3_600 },
    { label: '24 h', seconds: 86_400 },
  ];
  const allLevels: Level[] = [...LEVELS, 'unknown'];
  const WIDTH_KEY = 'ocekour.detail.width';

  const actions = new FieldActions();
  provideFieldActions(actions);

  // Session figée pour la vie du composant (l'application recrée la vue à chaque onglet).
  const s = untrack(() => session);
  const asm = s instanceof AssemblySession ? s : null;
  const isAssembly = !!asm;
  const rows = s.rows;

  const status = $derived(s.status());
  const search = $derived(s.search());
  const mode = $derived(s.searchMode());
  const options = $derived(s.options());
  const frame = $derived(s.version());
  const members = $derived(asm?.members() ?? []);
  const hidden = $derived(asm?.hiddenSources() ?? new Set<string>());
  const reorderMs = $derived(asm?.reorderMs() ?? 0);

  /** Réglages du workload affiché, issus des profils de config.yaml (neutres pour un assemblage). */
  const profile = $derived(s.target.kind === 'assembly' ? resolveProfile([], '') : resolveProfile(configService.config?.profiles ?? [], profileKey(s.target)));
  const fieldsKey = $derived(JSON.stringify(profile.fields));
  /** Logs de l'instance précédente disponibles (conteneur redémarré). */
  const hasPrevious = $derived(s.target.kind === 'container' && !!s.target.node.hasPrevious);
  /** Topic Kafka : colonnes partition / offset / clé, pas de « Précédent ». */
  const isTopic = s.target.kind === 'topic';
  const tailChoices = $derived([...new Set([...TAIL_CHOICES, configService.config?.defaultTailLines ?? 1_000])].sort((a, b) => a - b));
  const path = targetPath(s.target);

  const rowCount = $derived((frame, rows.length));
  const compiled = $derived.by((): { matcher?: Matcher; error?: string } => {
    try {
      return { matcher: buildMatcher(s.search()) };
    } catch (e) {
      return { error: (e as Error).message };
    }
  });
  const matcher = $derived(compiled.matcher);
  const searchError = $derived(compiled.error);
  /** Lignes par niveau : tenues à jour par le tampon, lues une fois par image. */
  const levelCounts = $derived((frame, { ...s.buffer.levelCounts } as Record<Level, number>));

  // --- Colonnes JSON indentées (profil : `wrap`) ---------------------------------------------
  const wrapColumns = $derived.by(() => {
    const shown = new Set([...profile.columns, MESSAGE_COLUMN]);
    return profile.wrap.filter((column) => shown.has(column));
  });
  const wrapKey = $derived(JSON.stringify([wrapColumns, profile.fields]));
  let cellCache = new WeakMap<LogEntry, { key: string; cells: Map<string, string | undefined>; lines: number }>();

  function cells(entry: LogEntry) {
    const key = wrapKey;
    let cached = cellCache.get(entry);
    if (!cached || cached.key !== key) {
      const fields = profile.fields;
      const map = new Map(wrapColumns.map((c) => [c, wrappedValue(entry, c, fields)] as const));
      let lines = 1;
      for (const text of map.values()) if (text) lines = Math.max(lines, text.split('\n').length);
      cached = { key, cells: map, lines };
      cellCache.set(entry, cached);
    }
    return cached;
  }

  /** JSON indenté de la cellule, ou `undefined` (colonne non dépliée, valeur non JSON). */
  const wrapped = (entry: LogEntry, column: string) => (wrapColumns.includes(column) ? cells(entry).cells.get(column) : undefined);
  const isWrapColumn = (column: string) => profile.wrap.includes(column);

  /** Notices regroupées dépliées (par numéro de ligne) : « afficher toutes les occurrences ». */
  let expandedNotices = $state.raw<ReadonlySet<number>>(new Set());

  function noticeLines(entry: LogEntry): number {
    const repeat = entry.repeat ?? 1;
    if (repeat <= 1 || !expandedNotices.has(entry.seq)) return 1;
    const shown = entry.times?.length ?? 0;
    return 1 + shown + (repeat > shown ? 1 : 0);
  }

  /** Hauteur d'une ligne : fixe, sauf cellules JSON indentées ou notice dépliée. */
  const rowHeightOf = (entry: LogEntry) => {
    const lines = entry.notice ? noticeLines(entry) : cells(entry).lines;
    return lines > 1 ? lines * LINE_HEIGHT + WRAP_PADDING : ROW_HEIGHT;
  };

  const matchCount = $derived((frame, rows.matchCount));
  const newLines = $derived((frame, s.stickToBottom() ? 0 : s.buffer.total - s.seqWhenUnstuck()));

  // --- Effets ---------------------------------------------------------------------------------
  let filterTimer: ReturnType<typeof setTimeout> | undefined;
  let wasPaused = false;

  $effect(() => {
    actions.profile.set(profile);
  });
  // Champs clés modifiés : les lignes déjà reçues sont ré-analysées (seulement s'ils ont changé).
  $effect(() => {
    const fields = JSON.parse(fieldsKey);
    untrack(() => {
      // Assemblage : chaque membre garde son profil (parsé à l'insertion).
      if (s instanceof AssemblySession) return;
      if (s.buffer.setFields(fields)) s.refresh();
    });
  });
  // Hauteurs variables : colonnes JSON indentées, ou notices dépliées.
  $effect(() => {
    const key = wrapKey;
    const expanded = expandedNotices;
    const needHeights = wrapColumns.length > 0 || expanded.size > 0;
    const fullKey = `${key}|${[...expanded].sort((a, b) => a - b).join(',')}`;
    untrack(() => {
      if (s.heightsKey === fullKey) return;
      s.heightsKey = fullKey;
      rows.setHeights(needHeights ? rowHeightOf : undefined, s.buffer);
      if (needHeights) rows.reset(s.buffer);
      s.invalidate();
    });
  });
  // Filtres : appliqués tout de suite, sauf le texte tapé (après une courte pause).
  $effect(() => {
    const levels = s.levels();
    const mode = s.searchMode();
    const search = s.search();
    const m = matcher;
    const rules = s.rules();
    const key = JSON.stringify([[...levels].sort(), mode, search, rules]);
    untrack(() => {
      if (key === s.filterKey) return;
      const textChanged = s.filterKey !== undefined && search.text !== s.filterText;
      s.filterKey = key;
      s.filterText = search.text;
      const apply = () =>
        s.setFilter({
          levels: levels.size === allLevels.length ? undefined : levels,
          matcher: mode === 'filter' ? m : undefined,
          highlight: mode === 'highlight' ? m : undefined,
          rules: compileRules(rules),
        });
      clearTimeout(filterTimer);
      if (textChanged) filterTimer = setTimeout(apply, SEARCH_DEBOUNCE_MS);
      else apply();
    });
  });
  // Reprise après une pause : l'affichage rattrape les lignes reçues.
  $effect(() => {
    const paused = s.paused();
    untrack(() => {
      if (!paused && wasPaused) s.refresh();
      wasPaused = paused;
    });
  });
  // « Filtrer sur cette valeur » demandé depuis le détail.
  $effect(() => {
    const request = actions.filterRequest();
    if (!request) return;
    untrack(() => {
      setSearch({ text: request.text, regex: false });
      s.searchMode.set('filter');
      actions.filterRequest.set(null);
    });
  });

  // --- Liste virtuelle --------------------------------------------------------------------------
  let viewport = $state<HTMLDivElement>();
  let searchInput = $state<HTMLInputElement>();
  let scrollTop = $state(0);
  let viewportHeight = $state(600);
  const total = $derived((frame, rows.total));
  const range = $derived((frame, total, visibleRange(rows, scrollTop, viewportHeight)));
  const visible = $derived.by(() => {
    void frame;
    const out: { entry: LogEntry; index: number; height: number }[] = [];
    for (let i = range.start; i < range.end; i++) {
      const entry = rows.at(i);
      if (entry) out.push({ entry, index: i, height: rows.heightOf(i) });
    }
    return out;
  });
  const offset = $derived((frame, rows.offsetOf(range.start)));

  onMount(() => {
    const el = viewport!;
    const observer = new ResizeObserver(() => (viewportHeight = el.clientHeight));
    observer.observe(el);
    viewportHeight = el.clientHeight;
    // Retour sur un onglet : on retrouve la position de lecture.
    if (!s.stickToBottom()) void tick().then(() => (el.scrollTop = s.scrollTop));
    return () => observer.disconnect();
  });
  onDestroy(() => clearTimeout(filterTimer));

  // Colle en bas tant que l'utilisateur ne remonte pas.
  $effect(() => {
    void frame;
    void total;
    if (s.stickToBottom() && rows.length && viewport) {
      viewport.scrollTop = viewport.scrollHeight;
    }
  });

  function onScroll() {
    const el = viewport;
    if (!el) return;
    scrollTop = el.scrollTop;
    s.scrollTop = el.scrollTop;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < ROW_HEIGHT * 2;
    if (atBottom !== s.stickToBottom()) {
      s.stickToBottom.set(atBottom);
      if (!atBottom) s.seqWhenUnstuck.set(s.buffer.total);
    }
  }

  function scrollToOffset(y: number) {
    if (viewport) viewport.scrollTop = y;
  }

  // --- Commandes --------------------------------------------------------------------------------
  function scrollToBottom() {
    s.stickToBottom.set(true);
    s.paused.set(false);
  }

  function setMode(follow: boolean) {
    if (follow !== s.options().follow) void s.restart({ follow });
  }

  const setTail = (value: string) => s.restart({ tailLines: value === 'all' ? undefined : Number(value) });
  const setSince = (value: string) => s.restart({ sinceSeconds: value ? Number(value) : undefined });
  const setPrevious = (previous: boolean) => s.restart({ previous });

  function clear() {
    s.buffer.clear();
    s.refresh();
  }

  function setSearch(patch: Partial<SearchOptions>) {
    s.search.update((v) => ({ ...v, ...patch }));
  }

  /** Clic : bascule le niveau ; Ctrl/Alt+clic : ce niveau seulement (ou tous). */
  function toggleLevel(level: Level, event: MouseEvent) {
    s.levels.update((current) => {
      if (event.ctrlKey || event.altKey || event.metaKey) {
        return current.size === 1 && current.has(level) ? new Set(allLevels) : new Set([level]);
      }
      const next = new Set(current);
      if (next.has(level)) next.delete(level);
      else next.add(level);
      return next;
    });
  }

  /** Exporte les lignes affichées (filtres compris) en NDJSON ou en texte. */
  async function exportLines() {
    const api = window.ocekour?.export;
    const list = rows.toArray();
    if (!api || !list.length) return;
    const target = s.target;
    const name = target.kind === 'container' ? `${target.pod}-${target.container}` : target.kind === 'topic' ? target.topic : target.kind === 'assembly' ? target.name || 'assemblage' : 'export';
    try {
      const pick = await api.pick(exportFileName(name));
      if (!pick) return;
      await api.write(pick.token, exportContent(list, pick.format));
      const count = list.filter((e) => !e.notice).length;
      notifications.info(`${count.toLocaleString('fr-FR')} ligne(s) exportée(s)`);
      analytics.event({ category: 'Export', action: pick.format, name: target.kind === 'container' ? 'rancher' : 'kafka', value: count });
    } catch (e) {
      notifications.error(e);
    }
  }

  /** F3 / Maj+F3 : correspondance suivante / précédente (mode « surligner »). */
  function nextMatch(direction: 1 | -1) {
    const m = matcher;
    if (!m || !rows.length) return;
    const current = rows.indexOfSeq(s.selectedSeq());
    const from = current >= 0 ? current : direction === 1 ? -1 : 0;
    const next = rows.find(from, direction, (e) => !e.notice && m.test(e.haystack));
    if (next >= 0) select(next);
  }

  /** Revoir la ligne sélectionnée au milieu du flux complet. */
  function showContext() {
    s.searchMode.set('highlight');
    s.levels.set(new Set(allLevels));
    requestAnimationFrame(() => {
      const index = rows.indexOfSeq(s.selectedSeq());
      if (index >= 0) select(index, true);
    });
  }

  function onGlobalKey(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
      event.preventDefault();
      searchInput?.focus();
      searchInput?.select();
    } else if (event.key === 'F3') {
      event.preventDefault();
      nextMatch(event.shiftKey ? -1 : 1);
    }
  }

  // --- Sélection --------------------------------------------------------------------------------
  const selected = $derived.by(() => {
    void frame;
    const seq = s.selectedSeq();
    return seq === null ? undefined : rows.at(rows.indexOfSeq(seq));
  });

  function select(index: number, center = false) {
    const entry = rows.at(index);
    if (!entry) return;
    s.selectedSeq.set(entry.seq);
    s.stickToBottom.set(false);
    s.seqWhenUnstuck.set(s.buffer.total);
    ensureVisible(index, center);
  }

  /** Clic sur une ligne : déplie une notice regroupée si le compteur est cliqué, sinon sélection. */
  function onRowClick(index: number, event: MouseEvent) {
    const toggle = (event.target as HTMLElement | null)?.closest('[data-notice-toggle]');
    if (toggle) {
      event.stopPropagation();
      const seq = Number(toggle.getAttribute('data-notice-toggle'));
      if (Number.isInteger(seq)) {
        const next = new Set(expandedNotices);
        if (next.has(seq)) next.delete(seq);
        else next.add(seq);
        expandedNotices = next;
      }
      return;
    }
    select(index);
  }

  function onKey(event: KeyboardEvent) {
    if (!rows.length) return;
    const page = Math.max(1, Math.floor((viewport?.clientHeight ?? 0) / ROW_HEIGHT) - 1);
    const current = rows.indexOfSeq(s.selectedSeq());
    const moves: Record<string, number> = {
      ArrowDown: current + 1,
      ArrowUp: current < 0 ? rows.length - 1 : current - 1,
      PageDown: current + page,
      PageUp: current - page,
      Home: 0,
      End: rows.length - 1,
    };
    if (event.key === 'Escape') {
      s.selectedSeq.set(null);
      return;
    }
    if (!(event.key in moves)) return;
    event.preventDefault();
    select(Math.min(Math.max(moves[event.key], 0), rows.length - 1));
  }

  function ensureVisible(index: number, center = false) {
    if (!viewport) return;
    if (center) {
      const middle = rows.offsetOf(index) + rows.heightOf(index) / 2;
      scrollToOffset(Math.max(0, middle - viewport.clientHeight / 2));
      return;
    }
    const top = viewport.scrollTop;
    const height = viewport.clientHeight;
    const y = rows.offsetOf(index);
    const h = rows.heightOf(index);
    if (y < top || h > height) scrollToOffset(y);
    else if (y + h > top + height) scrollToOffset(y + h - height);
  }

  // --- Panneau de détail ------------------------------------------------------------------------
  let detailWidth = $state(readWidth());

  function readWidth(): number {
    try {
      return Number(localStorage.getItem(WIDTH_KEY)) || 560;
    } catch {
      return 560;
    }
  }

  function startResize(start: PointerEvent) {
    const handle = start.target as HTMLElement;
    handle.setPointerCapture(start.pointerId);
    const initial = detailWidth;
    const move = (e: PointerEvent) => (detailWidth = Math.min(Math.max(initial + start.clientX - e.clientX, 280), window.innerWidth * 0.8));
    const up = () => {
      handle.removeEventListener('pointermove', move);
      try {
        localStorage.setItem(WIDTH_KEY, String(detailWidth));
      } catch {
        // stockage indisponible : largeur non mémorisée
      }
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up, { once: true });
  }

  // --- Rendu d'une ligne (mis en cache) -----------------------------------------------------------
  interface RowView {
    key: string;
    cls: string;
    match: boolean;
    html: string;
    repeat?: number;
    expanded?: boolean;
  }
  const viewCache = new WeakMap<LogEntry, RowView>();
  const viewKey = $derived(JSON.stringify([s.search(), s.searchMode(), profile.columns, wrapKey]));

  function row(entry: LogEntry): RowView {
    const key = viewKey;
    const expanded = !!entry.notice && expandedNotices.has(entry.seq);
    let view = viewCache.get(entry);
    if (view?.key === key && view.repeat === entry.repeat && view.expanded === expanded) return view;
    const m = matcher;
    const highlight = s.searchMode() === 'highlight';
    const messageJson = entry.notice ? undefined : wrapped(entry, MESSAGE_COLUMN);
    const message = messageJson ?? clip(entry.parsed.message);
    // Contenu de la ligne en un seul fragment HTML (textes échappés) : bien plus rapide à
    // afficher que de nombreux petits éléments par ligne pendant le défilement.
    let html = `<span class="w-24 shrink-0 text-slate-400 tabular-nums" title="${esc(fullDate(entry))}">${esc(formatTime(entry))}</span>`;
    if (isAssembly && entry.source) {
      const src = entry.source;
      html += `<span class="w-28 shrink-0 truncate font-semibold" style="color:${esc(src.color)}" title="${esc(src.label)}">${esc(src.label)}</span>`;
    } else if (isAssembly) {
      html += `<span class="w-28 shrink-0"></span>`;
    }
    if (entry.notice) {
      const repeat = entry.repeat ?? 1;
      const badge =
        repeat > 1
          ? ` <button type="button" data-notice-toggle="${entry.seq}" title="${expanded ? 'Replier' : 'Afficher toutes les occurrences'}" class="not-italic cursor-pointer rounded bg-sky-200 px-1 text-[10px] font-semibold text-sky-800 dark:bg-sky-800 dark:text-sky-100">×${repeat} ${expanded ? '▲' : '▼'}</button>`
          : '';
      if (expanded && repeat > 1) {
        html += `<span class="min-w-0 flex-1 italic leading-4">— ${esc(entry.text)} —${badge}`;
        for (const t of entry.times ?? []) html += `<br>${esc(noticeTime(t))}`;
        const shown = entry.times?.length ?? 0;
        if (repeat > shown) html += `<br>… (+${(repeat - shown).toLocaleString('fr-FR')})`;
        html += `</span>`;
      } else {
        html += `<span class="min-w-0 flex-1 truncate italic">— ${esc(entry.text)} —${badge}</span>`;
      }
    } else {
      const level = entry.parsed.level;
      html += `<span class="w-11 shrink-0 font-semibold ${LEVEL_CLASS[level]}">${level === 'unknown' ? '' : level.toUpperCase()}</span>`;
      const meta = isAssembly ? undefined : entry.meta;
      if (meta) {
        html +=
          `<span class="w-8 shrink-0 text-right text-slate-400 tabular-nums">${esc(String(meta.partition))}</span>` +
          `<span class="w-16 shrink-0 truncate text-right text-slate-400 tabular-nums" title="${esc(String(meta.offset))}">${esc(String(meta.offset))}</span>` +
          `<span class="w-28 shrink-0 truncate text-violet-700 dark:text-violet-300" title="${esc(meta.key ?? '')}">${esc(meta.key ?? '')}</span>`;
      }
      for (const col of profile.columns) {
        const json = wrapped(entry, col);
        html +=
          json !== undefined
            ? `<span class="w-96 shrink-0 overflow-hidden leading-4 text-slate-600 dark:text-slate-300" title="${esc(col)}">${esc(json)}</span>`
            : `<span class="${isWrapColumn(col) ? 'w-96' : 'w-36'} shrink-0 truncate text-slate-600 dark:text-slate-300" title="${esc(col)}">${esc(clip(columnText(entry, col)))}</span>`;
      }
      html += `<span class="min-w-0 flex-1 ${messageJson !== undefined ? 'overflow-hidden leading-4' : 'truncate'}">`;
      for (const seg of segments(message, m)) html += seg.match ? `<span class="hl">${esc(seg.text)}</span>` : esc(seg.text);
      html += '</span>';
    }
    view = { key, cls: baseClass(entry), match: highlight && !entry.notice && !!m?.test(entry.haystack), repeat: entry.repeat, expanded, html };
    viewCache.set(entry, view);
    return view;
  }

  // --- Filtres (encart de règles) -------------------------------------------------------------------
  let filtersOpen = $state(false);
  const rules = $derived(s.rules());
  const activeRuleCount = $derived(rules.filter((r) => r.enabled && r.value).length);

  function addRule() {
    s.rules.update((list) => [...list, newRule()]);
    filtersOpen = true;
  }
  const removeRule = (id: string) => s.rules.update((list) => list.filter((r) => r.id !== id));
  const updateRule = (id: string, patch: Partial<LogFilterRule>) => s.rules.update((list) => list.map((r) => (r.id === id ? { ...r, ...patch } : r)));

  // --- Filtres enregistrés -----------------------------------------------------------------------
  const presets = $derived(configService.filterPresets);
  let presetName = $state('');
  const canSavePreset = $derived(!!presetName.trim() && rules.some((r) => r.value));

  function applyPreset(name: string) {
    const preset = presets.find((p) => p.name === name);
    if (!preset) return;
    s.rules.set(preset.rules.map((r) => newRule(r)));
    presetName = preset.name;
    filtersOpen = true;
  }

  function saveCurrentPreset() {
    const name = presetName.trim();
    if (!name) return;
    void configService.saveFilterPreset(name, s.rules().map(({ id: _id, ...rule }) => rule));
  }

  function deletePreset(name: string) {
    if (!name) return;
    void configService.deleteFilterPreset(name);
    if (presetName === name) presetName = '';
  }

  // --- Téléchargement de l'historique complet ---------------------------------------------------
  let dlOpen = $state(false);

  function sourceOf(target: StreamTarget, label?: string): LogDownloadSource {
    return { connectionId: target.connectionId, path: streamPath(target).path, label, includePrevious: target.kind === 'container' && !!target.node.hasPrevious };
  }

  /** Historique complet : `filtered` applique les filtres courants, sinon tout. */
  async function download(filtered: boolean) {
    dlOpen = false;
    const api = window.ocekour?.export;
    if (!api) return;
    const search = filtered && s.search().text ? s.search() : undefined;
    const levels = filtered && s.levels().size < allLevels.length ? [...s.levels()] : undefined;
    let sources: LogDownloadSource[];
    let name: string;
    if (asm) {
      const list = asm.members().filter((m) => !filtered || !hidden.has(m.id));
      if (!list.length) {
        notifications.error('Aucune source à télécharger (toutes masquées).');
        return;
      }
      sources = list.map((m) => sourceOf(m.target, m.label));
      name = asm.target.name || 'assemblage';
    } else {
      const t = s.target as StreamTarget;
      sources = [sourceOf(t)];
      name = t.kind === 'container' ? `${t.pod}-${t.container}` : t.topic;
    }
    try {
      const result = await api.downloadLogs({ defaultName: name, filtered, search, levels, sources });
      if (!result) return;
      notifications.info(`${result.lines.toLocaleString('fr-FR')} ligne(s) téléchargée(s)` + (result.truncated ? ' (historique tronqué : limite atteinte)' : ''));
      analytics.event({ category: 'Export', action: filtered ? 'historique filtré' : 'historique complet', name: asm ? 'assemblage' : s.target.kind, value: result.lines });
    } catch (e) {
      notifications.error(e as Error);
    }
  }

  function setReorder(value: string) {
    const ms = Number(value);
    if (Number.isFinite(ms) && ms >= 0) asm?.reorderMs.set(Math.min(ms, 30_000));
  }

  const statusClass = (state: string) =>
    state === 'live'
      ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/50 dark:text-emerald-300'
      : state === 'error'
        ? 'bg-red-100 text-red-700 dark:bg-red-900/50 dark:text-red-300'
        : state === 'ended'
          ? 'bg-slate-200 text-slate-700 dark:bg-slate-800 dark:text-slate-300'
          : 'bg-amber-100 text-amber-700 dark:bg-amber-900/50 dark:text-amber-300';
  const statusLabel = (state: string) =>
    state === 'connecting' ? 'Connexion…' : state === 'live' ? (options.follow && !options.previous ? '● En direct' : 'Chargement…') : state === 'reconnecting' ? 'Reconnexion…' : state === 'ended' ? 'Terminé' : 'Erreur';
</script>

<svelte:window onkeydown={onGlobalKey} />

{#snippet wrapToggle(col: string, label: string)}
  <button
    type="button"
    class="rounded px-0.5 font-semibold {isWrapColumn(col)
      ? 'bg-sky-100 text-sky-700 dark:bg-sky-900 dark:text-sky-300'
      : 'text-slate-300 hover:text-sky-600 dark:text-slate-600 dark:hover:text-sky-400'}"
    aria-pressed={isWrapColumn(col)}
    aria-label={'JSON indenté dans la colonne ' + label}
    title={isWrapColumn(col) ? 'JSON indenté : cliquer pour l’afficher sur une ligne' : 'Afficher le JSON de cette colonne indenté, sur plusieurs lignes (enregistré dans le profil)'}
    onclick={() => actions.toggleWrap(col)}>{'{}'}</button
  >
{/snippet}

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-slate-200 px-3 py-2 text-sm dark:border-slate-800">
    <div class="min-w-0 truncate">
      <span class="text-slate-500">{path.slice(0, -1).join(' / ')} /</span>
      <strong> {path[path.length - 1]}</strong>
    </div>

    <div class="flex rounded-md border border-slate-300 p-0.5 dark:border-slate-700" role="radiogroup" aria-label="Mode">
      <button type="button" role="radio" class="seg" aria-checked={options.follow} onclick={() => setMode(true)}>Suivi</button>
      <button type="button" role="radio" class="seg" aria-checked={!options.follow} onclick={() => setMode(false)}>Historique</button>
    </div>

    <label class="flex items-center gap-1.5">
      Lignes
      <select class="input py-0.5" aria-label="Nombre de lignes" onchange={(e) => setTail(e.currentTarget.value)}>
        {#each tailChoices as n (n)}<option value={n} selected={options.tailLines === n}>{n.toLocaleString('fr-FR')}</option>{/each}
        <option value="all" selected={options.tailLines === undefined}>{isTopic ? 'Tout (depuis le début)' : 'Tout'}</option>
      </select>
    </label>

    <label class="flex items-center gap-1.5">
      Depuis
      <select class="input py-0.5" aria-label="Depuis" onchange={(e) => setSince(e.currentTarget.value)}>
        {#each SINCE_CHOICES as c (c.label)}<option value={c.seconds ?? ''} selected={options.sinceSeconds === c.seconds}>{c.label}</option>{/each}
      </select>
    </label>

    {#if !isTopic}
      <label class="flex items-center gap-1.5" class:opacity-50={!hasPrevious} title={hasPrevious ? 'Logs de l’instance précédente du conteneur' : 'Aucun redémarrage : pas de logs précédents'}>
        <input type="checkbox" checked={!!options.previous} disabled={!hasPrevious && !options.previous} onchange={(e) => setPrevious(e.currentTarget.checked)} />
        Précédent
      </label>
    {/if}

    {#if options.follow && !options.previous}
      <button type="button" class="btn" onclick={() => s.paused.set(!s.paused())}>{s.paused() ? '▶ Reprendre' : '⏸ Pause'}</button>
    {/if}
    <button type="button" class="btn" onclick={clear}>Effacer</button>
    <button type="button" class="btn" title="Relancer le flux" onclick={() => s.start()}>⟳</button>
    <button type="button" class="btn" title="Enregistrer les lignes affichées (filtres compris) en NDJSON ou en texte" disabled={!rowCount} onclick={exportLines}>Exporter…</button>
    <div class="relative">
      <button type="button" class="btn" title="Télécharger l'historique complet depuis la création du/des pod(s)" onclick={() => (dlOpen = !dlOpen)}>Télécharger…</button>
      {#if dlOpen}
        <div class="absolute z-20 mt-1 flex min-w-48 flex-col rounded-md border border-slate-200 bg-white text-sm shadow-lg dark:border-slate-700 dark:bg-slate-900">
          <button type="button" class="px-3 py-1.5 text-left hover:bg-slate-100 dark:hover:bg-slate-800" onclick={() => download(false)}>Tout l'historique</button>
          <button type="button" class="px-3 py-1.5 text-left hover:bg-slate-100 dark:hover:bg-slate-800" title="Historique complet, filtres actuels appliqués (recherche, niveaux, sources)" onclick={() => download(true)}
            >Historique filtré</button
          >
        </div>
      {/if}
    </div>
    <button type="button" class="btn" title="Copier un lien vers cette vue (sans secret), à coller dans Ocekour" onclick={() => share.copyLink(s)}>🔗 Lien</button>
    <button
      type="button"
      class="btn py-0.5 text-xs"
      title={'Workload ' + profile.key + '\nProfils appliqués : ' + (profile.matches.join(', ') || 'aucun')}
      onclick={oneditconfig}>Profil : {profile.matches[0] ?? 'aucun'}</button
    >

    <div class="ml-auto flex items-center gap-2 text-xs text-slate-500">
      <span class="inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 font-medium {statusClass(status.state)}">{statusLabel(status.state)}</span>
      <span>{rowCount.toLocaleString('fr-FR')} lignes</span>
      {#if options.follow && status.state === 'live'}<span title="Lignes reçues par seconde">· {s.rate()}/s</span>{/if}
      {#if (frame, s.buffer.dropped)}<span title="Retirées de la mémoire (taille du tampon)">· {s.buffer.dropped.toLocaleString('fr-FR')} retirées</span>{/if}
      {#if s.paused()}<span class="font-medium text-amber-600">· en pause</span>{/if}
    </div>
  </div>

  {#if status.message && status.state !== 'live'}
    <div
      role="status"
      class="flex flex-wrap items-center gap-2 border-b px-3 py-1.5 text-sm {status.state === 'error'
        ? 'border-red-200 bg-red-50 text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-300'
        : 'border-slate-200 bg-slate-50 text-slate-600 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300'}"
    >
      <span>{status.message}</span>
      {#each status.replacements ?? [] as pod (pod)}
        <button type="button" class="btn py-0" onclick={() => onswitchpod(pod)}>Ouvrir {pod}</button>
      {/each}
    </div>
  {/if}

  <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-slate-200 px-3 py-1.5 text-sm dark:border-slate-800">
    <div class="flex min-w-64 flex-1 items-center gap-1">
      <input
        bind:this={searchInput}
        type="search"
        class="input min-w-0 flex-1 font-mono"
        class:border-red-500!={!!searchError}
        placeholder="Rechercher (Ctrl+F)…"
        aria-label="Rechercher dans les logs"
        title={searchError ?? ''}
        value={search.text}
        oninput={(e) => setSearch({ text: e.currentTarget.value })}
        onkeydown={(e) => {
          if (e.key === 'Enter' && mode === 'highlight') nextMatch(e.shiftKey ? -1 : 1);
          else if (e.key === 'Escape') setSearch({ text: '' });
        }}
      />
      <button type="button" class="seg" title="Respecter la casse" aria-checked={search.caseSensitive} role="switch" onclick={() => setSearch({ caseSensitive: !search.caseSensitive })}>Aa</button>
      <button type="button" class="seg font-mono" title="Expression régulière" aria-checked={search.regex} role="switch" onclick={() => setSearch({ regex: !search.regex })}>.*</button>
    </div>

    <div class="flex rounded-md border border-slate-300 p-0.5 dark:border-slate-700" role="radiogroup" aria-label="Mode de recherche">
      <button type="button" role="radio" class="seg" aria-checked={mode === 'filter'} onclick={() => s.searchMode.set('filter')} title="Masquer les lignes qui ne correspondent pas">Filtrer</button>
      <button type="button" role="radio" class="seg" aria-checked={mode === 'highlight'} onclick={() => s.searchMode.set('highlight')} title="Tout afficher, surligner les correspondances">Surligner</button>
    </div>

    {#if mode === 'highlight' && matcher}
      <span class="flex items-center gap-1 text-xs text-slate-500">
        {matchCount.toLocaleString('fr-FR')} résultat(s)
        <button type="button" class="btn px-1.5 py-0" title="Précédent (Maj+F3)" onclick={() => nextMatch(-1)}>↑</button>
        <button type="button" class="btn px-1.5 py-0" title="Suivant (F3)" onclick={() => nextMatch(1)}>↓</button>
      </span>
    {/if}
    {#if mode === 'filter' && selected && (matcher || s.levels().size < allLevels.length)}
      <button type="button" class="btn py-0.5 text-xs" title="Revoir cette ligne dans le flux complet" onclick={showContext}>Afficher le contexte</button>
    {/if}

    <div class="flex flex-wrap items-center gap-1 text-xs" role="group" aria-label="Niveaux (Ctrl+clic : celui-ci seulement)">
      {#each allLevels as level (level)}
        {#if levelCounts[level] || !s.levels().has(level)}
          <button
            type="button"
            class="rounded-full border px-2 py-0.5 tabular-nums {s.levels().has(level)
              ? 'border-slate-300 dark:border-slate-600 ' + LEVEL_CLASS[level]
              : 'border-dashed border-slate-300 text-slate-400 line-through dark:border-slate-700'}"
            aria-pressed={s.levels().has(level)}
            title="Clic : afficher / masquer · Ctrl+clic : ce niveau seulement"
            onclick={(e) => toggleLevel(level, e)}
          >
            {level === 'unknown' ? 'autre' : level.toUpperCase()}
            {(levelCounts[level] ?? 0).toLocaleString('fr-FR')}
          </button>
        {/if}
      {/each}
    </div>

    <button
      type="button"
      class="seg text-xs"
      role="switch"
      aria-checked={filtersOpen}
      title={filtersOpen ? 'Replier les filtres' : 'Masquer / garder des lignes par champ (exact ou regex)'}
      onclick={() => (filtersOpen = !filtersOpen)}
    >
      <span class="mr-0.5 inline-block text-[10px]">{filtersOpen ? '▾' : '▸'}</span>
      Filtres
      {#if activeRuleCount}<span class="ml-1 rounded-full bg-sky-600 px-1.5 text-[10px] font-semibold text-white">{activeRuleCount}</span>{/if}
    </button>
  </div>

  {#if filtersOpen}
    <div class="flex max-h-[45vh] flex-col gap-1.5 overflow-y-auto border-b border-slate-200 px-3 py-2 text-xs dark:border-slate-800">
      <div class="flex flex-wrap items-center gap-1.5 border-b border-slate-100 pb-1.5 dark:border-slate-800/60">
        <span class="text-slate-500">Filtre enregistré :</span>
        <select class="input w-44 py-0.5" aria-label="Charger un filtre enregistré" value={presetName} onchange={(e) => applyPreset(e.currentTarget.value)}>
          <option value="">— aucun —</option>
          {#each presets as preset (preset.name)}<option value={preset.name}>{preset.name}</option>{/each}
        </select>
        {#if presetName && presets.length}
          <button type="button" class="btn px-1.5 py-0.5 text-slate-400 hover:text-red-500" title="Supprimer ce filtre enregistré" onclick={() => deletePreset(presetName)}>Supprimer</button>
        {/if}
        <span class="mx-1 text-slate-300 dark:text-slate-700">|</span>
        <input class="input w-40 py-0.5" placeholder="nom du filtre" aria-label="Nom du filtre à enregistrer" bind:value={presetName} onkeydown={(e) => e.key === 'Enter' && saveCurrentPreset()} />
        <button type="button" class="btn py-0.5" title="Enregistrer les règles actuelles sous ce nom (réutilisable dans tous les onglets)" disabled={!canSavePreset} onclick={saveCurrentPreset}>Enregistrer</button>
      </div>
      {#each rules as rule (rule.id)}
        <div class="flex flex-wrap items-center gap-1.5">
          <input type="checkbox" checked={rule.enabled} aria-label="Règle active" onchange={(e) => updateRule(rule.id, { enabled: e.currentTarget.checked })} />
          <select class="input py-0.5" value={rule.action} aria-label="Action" onchange={(e) => updateRule(rule.id, { action: e.currentTarget.value as LogFilterRule['action'] })}>
            <option value="hide">Masquer</option>
            <option value="only">Garder seul</option>
          </select>
          <input class="input w-40 py-0.5 font-mono" list="filtre-champs" placeholder="champ (ex. path)" value={rule.field} aria-label="Champ" onchange={(e) => updateRule(rule.id, { field: e.currentTarget.value })} />
          <select class="input py-0.5" value={rule.mode} aria-label="Comparaison" onchange={(e) => updateRule(rule.id, { mode: e.currentTarget.value as LogFilterRule['mode'] })}>
            <option value="contains">contient</option>
            <option value="exact">exact</option>
            <option value="regex">regex</option>
          </select>
          <input
            class="input min-w-0 flex-1 py-0.5 font-mono"
            class:border-red-500!={!!ruleError(rule)}
            placeholder="valeur"
            title={ruleError(rule) ?? ''}
            value={rule.value}
            aria-label="Valeur"
            oninput={(e) => updateRule(rule.id, { value: e.currentTarget.value })}
          />
          <button type="button" class="seg" role="switch" title="Respecter la casse" aria-checked={rule.caseSensitive} onclick={() => updateRule(rule.id, { caseSensitive: !rule.caseSensitive })}>Aa</button>
          <button type="button" class="btn px-1.5 py-0.5 text-slate-400 hover:text-red-500" title="Supprimer la règle" onclick={() => removeRule(rule.id)}>✕</button>
        </div>
      {/each}
      <datalist id="filtre-champs">
        <option value="@message"></option>
        <option value="@line"></option>
        {#each profile.columns as col (col)}<option value={col}></option>{/each}
      </datalist>
      <div class="flex items-center gap-2">
        <button type="button" class="btn py-0.5" onclick={addRule}>+ Ajouter une règle</button>
        {#if rules.length}<button type="button" class="btn py-0.5" onclick={() => s.rules.set([])}>Tout effacer</button>{/if}
        <span class="text-slate-500">Masquer = liste noire · Garder seul = liste blanche · champ <code>path</code>, <code>@message</code> (message) ou <code>@line</code> (ligne brute).</span>
      </div>
    </div>
  {/if}

  {#if asm}
    <div class="flex flex-wrap items-center gap-2 border-b border-slate-200 px-3 py-1.5 text-xs dark:border-slate-800">
      <input class="input w-40 py-0.5" placeholder="Assemblage" value={asm.target.name ?? ''} aria-label="Nom de l'assemblage" onchange={(e) => asm.rename(e.currentTarget.value)} />
      <span class="text-slate-500">Sources :</span>
      {#each members as m (m.id)}
        <span class="inline-flex items-center gap-1 rounded border px-1.5 py-0.5" style:border-color={m.color}>
          <input type="checkbox" checked={!hidden.has(m.id)} aria-label={'Afficher ' + m.label} onchange={() => asm.toggleSource(m.id)} />
          <span class="inline-block h-2 w-2 shrink-0 rounded-full" style:background={m.color}></span>
          <span class="max-w-40 truncate" title={m.label}>{m.label}</span>
          <button type="button" class="text-slate-400 hover:text-red-500" title={'Retirer ' + m.label} onclick={() => asm.removeMember(m.id)}>✕</button>
        </span>
      {/each}
      {#if !members.length}<span class="text-slate-500 italic">Cliquez un conteneur ou un topic dans la liste de gauche pour l'ajouter.</span>{/if}
      <label class="ml-auto flex items-center gap-1.5 text-slate-500" title="0 = ordre d'arrivée">
        Réordonner (ms)
        <input type="number" class="input w-20 py-0.5" min="0" max="30000" step="500" value={reorderMs} onchange={(e) => setReorder(e.currentTarget.value)} />
      </label>
    </div>
  {/if}

  <div class="flex min-h-0 flex-1">
    <div class="relative flex min-w-0 flex-1 flex-col">
      <div class="flex h-6 shrink-0 items-center gap-3 overflow-hidden border-b border-slate-200 bg-slate-50 px-3 font-mono text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-900">
        <span class="w-24 shrink-0">heure</span>
        {#if isAssembly}<span class="w-28 shrink-0">source</span>{/if}
        <span class="w-11 shrink-0">niveau</span>
        {#if isTopic}
          <span class="w-8 shrink-0 text-right" title="Partition">part.</span>
          <span class="w-16 shrink-0 text-right">offset</span>
          <span class="w-28 shrink-0">clé</span>
        {/if}
        {#each profile.columns as col (col)}
          <span class="group/col flex shrink-0 items-center gap-1 truncate {isWrapColumn(col) ? 'w-96' : 'w-36'}">
            <span class="truncate" title={col}>{col}</span>
            {@render wrapToggle(col, col)}
            <button type="button" class="hidden text-slate-400 group-hover/col:inline hover:text-red-500" title={'Retirer la colonne ' + col} onclick={() => actions.toggleColumn(col)}>✕</button>
          </span>
        {/each}
        <span class="group/col flex flex-1 items-center gap-1">message {@render wrapToggle(MESSAGE_COLUMN, 'message')}</span>
      </div>

      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div bind:this={viewport} tabindex="0" role="log" aria-label="Lignes de log" class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto font-mono text-xs focus-visible:outline-none" onscroll={onScroll} onkeydown={onKey}>
        <div class="relative w-full" style:height="{total}px">
          <div class="absolute inset-x-0 top-0" style:transform="translateY({offset}px)">
            {#each visible as item (item.entry.seq)}
              {@const v = row(item.entry)}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div
                class="flex cursor-default gap-3 px-3 whitespace-pre [contain:strict] {item.entry.seq === s.selectedSeq() ? 'bg-sky-100 dark:bg-sky-900/60' : v.cls}"
                class:items-center={item.height === ROW_HEIGHT}
                class:items-start={item.height !== ROW_HEIGHT}
                class:match-row={v.match}
                style:height="{item.height}px"
                style:padding-block={item.height !== ROW_HEIGHT ? '2px' : null}
                onclick={(e) => onRowClick(item.index, e)}
              >
                {@html v.html}
              </div>
            {/each}
          </div>
        </div>
      </div>

      {#if newLines > 0}
        <button type="button" class="btn-primary absolute right-6 bottom-4 shadow-lg" onclick={scrollToBottom}>↓ {newLines.toLocaleString('fr-FR')} nouvelle(s) ligne(s)</button>
      {/if}
    </div>
    {#if selected}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="w-1 shrink-0 cursor-col-resize bg-slate-200 hover:bg-sky-400 dark:bg-slate-800" role="separator" aria-orientation="vertical" aria-label="Redimensionner le détail" onpointerdown={startResize}></div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="flex min-h-0 shrink-0 flex-col bg-white dark:bg-slate-950" style:width="{detailWidth}px" onkeydown={(e) => e.key === 'Escape' && s.selectedSeq.set(null)}>
        <LogDetail entry={selected} highlight={matcher} expand={profile.expand} parseNestedDefault={profile.parseNested} onclose={() => s.selectedSeq.set(null)} />
      </div>
    {/if}
  </div>
</div>
