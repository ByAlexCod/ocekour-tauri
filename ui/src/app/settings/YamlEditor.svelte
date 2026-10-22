<script lang="ts">
  import { yaml } from '@codemirror/lang-yaml';
  import { type Diagnostic, linter, lintGutter } from '@codemirror/lint';
  import { Compartment, type Text } from '@codemirror/state';
  import { oneDark } from '@codemirror/theme-one-dark';
  import { EditorView, keymap } from '@codemirror/view';
  import { basicSetup } from 'codemirror';
  import { onMount } from 'svelte';
  import type { ConfigError } from '../../../shared/api';
  import { configService } from '../core/config.svelte';
  import { notifications } from '../core/notification.svelte';
  import { themeService } from '../core/theme.svelte';

  /** Éditeur de config.yaml : coloration, erreurs en ligne (validées par le cœur), Ctrl+S. */
  const theme = new Compartment();
  let host: HTMLDivElement;
  let view = $state<EditorView>();
  let savedText = '';
  let dirty = $state(false);

  onMount(() => {
    view = new EditorView({
      parent: host,
      extensions: [
        basicSetup,
        yaml(),
        lintGutter(),
        linter((v) => lint(v.state.doc), { delay: 300 }),
        keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => (void save(), true) }]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) dirty = u.state.doc.toString() !== savedText;
        }),
        EditorView.theme({ '&': { height: '100%' }, '.cm-scroller': { overflow: 'auto' } }),
        theme.of(themeService.dark ? oneDark : []),
      ],
    });
    void reload().then(() => view?.focus());
    // Fichier modifié hors de l'app : on recharge si l'utilisateur n'a rien en cours.
    const stop = configService.api?.onChanged(() => {
      if (!dirty) void reload();
    });
    return () => {
      stop?.();
      view?.destroy();
    };
  });

  $effect(() => {
    const dark = themeService.dark;
    view?.dispatch({ effects: theme.reconfigure(dark ? oneDark : []) });
  });

  async function reload() {
    const text = (await configService.api?.readText()) ?? '';
    savedText = text;
    view?.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    dirty = false;
  }

  async function save() {
    const api = configService.api;
    if (!view || !api) return;
    const text = view.state.doc.toString();
    try {
      const result = await api.saveText(text);
      if (!result.ok) {
        notifications.error(`Non enregistré : ${result.errors[0].message}`);
        return;
      }
      savedText = text;
      dirty = false;
      notifications.info('config.yaml enregistré');
    } catch (e) {
      notifications.error(e);
    }
  }

  async function lint(doc: Text): Promise<Diagnostic[]> {
    const errors = (await configService.api?.validateText(doc.toString())) ?? [];
    return errors.map((e) => toDiagnostic(e, doc));
  }

  function toDiagnostic(error: ConfigError, doc: Text): Diagnostic {
    const line = doc.line(Math.min(Math.max(error.line ?? 1, 1), doc.lines));
    const from = Math.min(line.from + Math.max((error.col ?? 1) - 1, 0), line.to);
    return { from, to: line.to, severity: 'error', message: error.message };
  }
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex items-center gap-2 border-b border-slate-200 px-3 py-2 text-sm dark:border-slate-800">
    <button type="button" class="btn-primary" onclick={save} disabled={!dirty}>Enregistrer <kbd class="opacity-60">Ctrl+S</kbd></button>
    <button type="button" class="btn" onclick={reload}>Recharger depuis le disque</button>
    <button type="button" class="btn" onclick={() => configService.showFile()}>Afficher le fichier</button>
    <span class="ml-auto text-slate-500">{dirty ? 'Modifications non enregistrées' : configService.state?.path}</span>
  </div>
  <div bind:this={host} class="min-h-0 flex-1 overflow-hidden"></div>
</div>
