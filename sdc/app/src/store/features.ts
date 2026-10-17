import { create } from 'zustand';

import type { FeaturesView } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { storedSettings, storeSetting } from '../lib/settings';
import { refreshCatalog } from './intents';

/**
 * The switches and the profile (Phase 4.4), and Simple/Pro. Simple and Pro are the same app: Pro only shows the Advanced tab, where every
 * switch can be read and flipped. A profile is a named setting of those switches (Safe, Balanced, Economical); `custom` is what it says
 * when switches were set by hand.
 */
export type UiMode = 'simple' | 'pro';

interface FeaturesState {
  mode: UiMode;
  view: FeaturesView | null;
  failed: boolean;
}

const initialMode = (): UiMode => (storedSettings()['ui-mode'] === 'pro' ? 'pro' : 'simple');

export const useFeatures = create<FeaturesState>()(() => ({ mode: initialMode(), view: null, failed: false }));

export function setUiMode(mode: UiMode): void {
  storeSetting('ui-mode', mode);
  useFeatures.setState({ mode });
}

export async function loadFeatures(): Promise<void> {
  try {
    useFeatures.setState({ view: await sdcpCall('features.list', {}), failed: false });
  } catch {
    useFeatures.setState({ failed: true });
  }
}

export async function applyPreset(preset: 'safe' | 'balanced' | 'economical'): Promise<void> {
  try {
    useFeatures.setState({ view: await sdcpCall('features.preset', { preset }), failed: false });
    void refreshCatalog();
  } catch {
    useFeatures.setState({ failed: true });
  }
}

/** One switch, by hand. The setting is written by the daemon (`settings.set` accepts `v3.*`) and the list is read again. */
export async function setSwitch(name: string, on: boolean): Promise<void> {
  try {
    await sdcpCall('settings.set', { key: `v3.${name}`, value: on ? 'on' : 'off' });
    await loadFeatures();
  } catch {
    useFeatures.setState({ failed: true });
  }
}
