/** Opens a URL in the person's own browser - through Tauri's shell in the app, a new tab in dev. */
export async function openOutside(url: string): Promise<void> {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');

    await open(url);
  } catch {
    window.open(url, '_blank', 'noopener');
  }
}
