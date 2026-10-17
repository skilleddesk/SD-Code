import { create } from 'zustand';

import type { ShipReport } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { storedSettings, storeSetting } from '../lib/settings';
import { strings } from '../strings';
import { usePrefsStore } from './prefs';
import { findSession } from './sessions';
import { useAppStore } from './store';

/** Ship Safe (Phase 4.5): the five pre-launch checks, run on the open folder and (optionally) a site address. */
interface ShipSafeState {
  open: boolean;
  running: boolean;
  report: ShipReport | null;
  error: string | null;
  url: string;
}

const savedUrl = (): string => {
  const value = storedSettings()['ship-url'];

  return typeof value === 'string' ? value : '';
};

export const useShipSafe = create<ShipSafeState>()(() => ({ open: false, running: false, report: null, error: null, url: savedUrl() }));

export function openShipSafe(): void {
  useShipSafe.setState({ open: true, error: null });
}

export function closeShipSafe(): void {
  useShipSafe.setState({ open: false });
}

export function setShipUrl(url: string): void {
  storeSetting('ship-url', url);
  useShipSafe.setState({ url });
}

/** The folder of the open chat, when it is one on this computer. */
export function shipRoot(): string | null {
  const activeTab = usePrefsStore.getState().activeTab;
  const found = activeTab === null ? null : findSession(useAppStore.getState().hosts, activeTab);

  /* only a folder on this computer: Ship Safe reads files, and a server's folder is not here */
  return found !== null && found.host.type === 'local' ? (found.session.projectRoot ?? null) : null;
}

export async function runShipSafe(): Promise<void> {
  const root = shipRoot();

  if (root === null) {
    useShipSafe.setState({ error: strings.shipSafe.noFolder });

    return;
  }

  const { url } = useShipSafe.getState();

  useShipSafe.setState({ running: true, error: null, report: null });

  try {
    const report = await sdcpCall('ship.check', { path: root, ...(url.trim() === '' ? {} : { url: url.trim() }) });

    useShipSafe.setState({ running: false, report });
  } catch (reason) {
    useShipSafe.setState({ running: false, error: isSdcpError(reason) ? reason.message : strings.shipSafe.failed });
  }
}
