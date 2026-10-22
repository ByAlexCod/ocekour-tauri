<script lang="ts">
  import { notifications } from '../core/notification.svelte';
</script>

<div class="fixed right-4 bottom-4 z-50 flex flex-col items-end gap-2">
  {#each notifications.toasts as t (t.id)}
    <div
      role="status"
      class="flex max-w-md items-start gap-3 rounded-md px-3 py-2 text-sm shadow-lg {t.kind === 'error'
        ? 'bg-red-600 text-white'
        : 'bg-slate-800 text-white'}"
    >
      <span class="flex-1 whitespace-pre-wrap">{t.text}</span>
      <button type="button" aria-label="Fermer" class="opacity-70 hover:opacity-100" onclick={() => notifications.dismiss(t.id)}>✕</button>
    </div>
  {/each}
</div>
