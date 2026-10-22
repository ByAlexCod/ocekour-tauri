<script lang="ts">
  import { json } from '@codemirror/lang-json';
  import { xml } from '@codemirror/lang-xml';
  import { yaml } from '@codemirror/lang-yaml';
  import { Compartment, EditorState, type Extension } from '@codemirror/state';
  import { oneDark } from '@codemirror/theme-one-dark';
  import { EditorView } from '@codemirror/view';
  import { basicSetup } from 'codemirror';
  import { onMount } from 'svelte';
  import { themeService } from '../core/theme.svelte';
  import type { PreviewLanguage } from './format';

  /** Aperçu texte en lecture seule : coloration, recherche (Ctrl+F), grands fichiers. */
  let { text, language = 'text' }: { text: string; language?: PreviewLanguage } = $props();

  const LANGUAGES: Record<PreviewLanguage, () => Extension> = { json: () => json(), xml: () => xml(), yaml: () => yaml(), text: () => [] };
  const theme = new Compartment();
  const lang = new Compartment();
  let host: HTMLDivElement;
  let view = $state<EditorView>();

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: text,
        extensions: [
          basicSetup,
          EditorState.readOnly.of(true),
          EditorView.lineWrapping,
          lang.of(LANGUAGES[language]()),
          EditorView.theme({ '&': { height: '100%', fontSize: '12px' }, '.cm-scroller': { overflow: 'auto' } }),
          theme.of(themeService.dark ? oneDark : []),
        ],
      }),
    });
    return () => view?.destroy();
  });

  $effect(() => {
    const t = text;
    const l = language;
    if (!view || view.state.doc.toString() === t) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: t }, effects: lang.reconfigure(LANGUAGES[l]()), selection: { anchor: 0 }, scrollIntoView: true });
  });

  $effect(() => {
    const dark = themeService.dark;
    view?.dispatch({ effects: theme.reconfigure(dark ? oneDark : []) });
  });
</script>

<div class="block min-h-0 flex-1 overflow-hidden"><div bind:this={host} class="h-full"></div></div>
