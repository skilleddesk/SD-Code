import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

/**
 * Vite is driven by the Tauri CLI, so this config follows the Tauri 2 Vite guide:
 *
 * - a fixed dev port (Tauri's `build.devUrl` must match it exactly),
 * - `clearScreen: false` so Rust compiler output is not wiped,
 * - `src-tauri/**` is ignored by the watcher to avoid a dev-server reload loop.
 *
 * `TAURI_DEV_HOST` is set by Tauri when developing against a device/VM; it stays
 * undefined for normal desktop development.
 */
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
});
