import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';

// Interface Tauri : servie par Vite en développement (port fixe attendu par tauri.conf.json),
// construite dans `dist/` pour l'exe.
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022', outDir: 'dist', emptyOutDir: true, chunkSizeWarningLimit: 2_000 },
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/**/*.spec.ts'],
  },
  resolve: process.env['VITEST'] ? { conditions: ['browser'] } : undefined,
});
