/**
 * The desktop app draws its own title bar on every platform (0.21, `tauri.conf.json` `decorations: false`):
 * the same dark bar with the same three buttons on Windows, macOS and Linux. In a plain browser (the web
 * app, `pnpm dev`) the browser is the frame, so nothing is drawn.
 */
export function framelessWindow(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
