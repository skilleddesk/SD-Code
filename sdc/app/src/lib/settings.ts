/**
 * The Settings dialog's switches, remembered (0.11.8).
 *
 * The dialog kept its values in component state, so every switch went back to its default the next time
 * it opened - and nothing outside the dialog could read one. These are UI preferences, so they live in
 * localStorage (the rule in `store/prefs.ts`); a storage that refuses is not worth failing a click over.
 */
const KEY = 'sdc.settings.v1';

export type SettingValue = boolean | string;

export function storedSettings(): Record<string, SettingValue> {
  try {
    const raw = globalThis.localStorage?.getItem(KEY);

    return raw === null || raw === undefined ? {} : (JSON.parse(raw) as Record<string, SettingValue>);
  } catch {
    return {};
  }
}

export function storeSetting(id: string, value: SettingValue): void {
  try {
    globalThis.localStorage?.setItem(KEY, JSON.stringify({ ...storedSettings(), [id]: value }));
  } catch {
    /* Private mode or a disabled storage API: the switch still works for this session. */
  }
}

/** A switch's value: what was chosen, or its default. */
export function setting(id: string, fallback: boolean): boolean {
  const value = storedSettings()[id];

  return typeof value === 'boolean' ? value : fallback;
}
