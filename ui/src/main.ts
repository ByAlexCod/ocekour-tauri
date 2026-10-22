import { mount } from 'svelte';
import App from './app/App.svelte';
import { startServices } from './app/bootstrap';
import { notifications } from './app/core/notification.svelte';
import { httpApi, installDropTracking, isHttpMode, tauriApi } from './bridge';
import './styles.css';

// Dans la fenêtre Tauri ou en mode Edge (WebView2 indisponible), `window.ocekour` parle au
// cœur Rust ; ailleurs (navigateur de développement), l'interface s'affiche sans données.
if ('__TAURI_INTERNALS__' in window) {
  window.ocekour = tauriApi();
  installDropTracking();
} else if (isHttpMode()) {
  window.ocekour = httpApi();
  // Un fichier déposé ouvrirait le fichier dans Edge à la place d'Ocekour : le navigateur ne
  // donne pas son chemin, il s'ouvre par Ctrl+O.
  window.addEventListener('dragover', (e) => e.preventDefault());
  window.addEventListener('drop', (e) => {
    e.preventDefault();
    if (e.dataTransfer?.files.length) {
      notifications.info('Glisser-déposer indisponible ici (Ocekour affiché par Edge) : utilisez Ctrl+O, ou « Envoyer… » dans S3.');
    }
  });
}

// Erreurs non gérées : écrites dans le journal du cœur.
const report = (message: string) => void window.ocekour?.logError(message).catch(() => undefined);
window.addEventListener('error', (e) => report(e.error?.stack ?? e.message));
window.addEventListener('unhandledrejection', (e) => report(e.reason?.stack ?? String(e.reason)));

startServices();
mount(App, { target: document.getElementById('app')! });
