import { notifications } from './notification.svelte';

export async function copy(text: string, what = 'Copié') {
  try {
    await (window.ocekour?.copyText(text) ?? navigator.clipboard.writeText(text));
    notifications.info(what);
  } catch (e) {
    notifications.error(e);
  }
}
