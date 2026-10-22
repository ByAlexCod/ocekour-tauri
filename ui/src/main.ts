import { mount } from 'svelte';
import App from './app/App.svelte';
import { startServices } from './app/bootstrap';
import { installDropTracking, tauriApi } from './bridge';
import './styles.css';

// Dans la fenêtre Tauri, `window.ocekour` parle au cœur Rust ; ailleurs (navigateur de
// développement), l'interface s'affiche sans données.
if ('__TAURI_INTERNALS__' in window) {
  window.ocekour = tauriApi();
  installDropTracking();
}

// Erreurs non gérées : écrites dans le journal du cœur.
const report = (message: string) => void window.ocekour?.logError(message).catch(() => undefined);
window.addEventListener('error', (e) => report(e.error?.stack ?? e.message));
window.addEventListener('unhandledrejection', (e) => report(e.reason?.stack ?? String(e.reason)));

startServices();
mount(App, { target: document.getElementById('app')! });
