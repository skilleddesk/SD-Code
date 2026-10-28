import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/**
 * The app's own update (1.0.0): checked, downloaded, signature-checked and installed by the Tauri updater in the
 * desktop shell (`app_update_check` / `app_update_install` in src-tauri/src/lib.rs), on every platform. In a
 * browser there is no shell, and `inShell()` is false.
 */
export interface AppUpdate {
  available: boolean;
  current: string;
  version?: string;
  notes?: string | null;
}

export function inShell(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export async function checkAppUpdate(): Promise<AppUpdate> {
  return invoke<AppUpdate>('app_update_check');
}

/** Installs the update; `onProgress` hears the download as a percentage (`null` while the size is unknown). */
export async function installAppUpdate(onProgress: (percent: number | null) => void): Promise<void> {
  const stop = await listen<{ downloaded: number; total: number | null }>('app-update://progress', (event) => {
    const { downloaded, total } = event.payload;

    onProgress(total ? Math.min(100, Math.floor((downloaded / total) * 100)) : null);
  });

  try {
    await invoke('app_update_install');
  } finally {
    stop();
  }
}
