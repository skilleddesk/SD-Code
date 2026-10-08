import type { AnywhereDevice, AnywherePairRequest, AnywhereStatus } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';
import { toast } from './toast';

/**
 * Settings → SDC Anywhere talks to the daemon only through these. Each wraps one `anywhere.*` method; the
 * daemon refuses all of them to a browser, so only this window can change the feature.
 */
function failed(error: unknown): void {
  toast(isSdcpError(error) ? error.message : strings.anywhere.failed);
}

export async function anywhereStatus(): Promise<AnywhereStatus | null> {
  try {
    return await sdcpCall('anywhere.status', {});
  } catch {
    return null;
  }
}

export async function setAnywhere(on: boolean): Promise<AnywhereStatus | null> {
  try {
    const status = await sdcpCall(on ? 'anywhere.enable' : 'anywhere.disable', {});

    toast(on ? strings.anywhere.enabledToast : strings.anywhere.disabledToast);

    return status;
  } catch (error) {
    failed(error);

    return null;
  }
}

export async function configureAnywhere(params: Parameters<typeof sdcpCall<'anywhere.configure'>>[1]): Promise<AnywhereStatus | null> {
  try {
    const status = await sdcpCall('anywhere.configure', params);

    toast(strings.anywhere.settings.saved);

    return status;
  } catch (error) {
    failed(error);

    return null;
  }
}

export async function beginPairing(guest: boolean): Promise<{ url: string; fingerprint: string; expiresAt: number } | null> {
  try {
    return await sdcpCall('anywhere.pair.begin', { guest });
  } catch (error) {
    failed(error);

    return null;
  }
}

export async function pairRequests(): Promise<AnywherePairRequest[]> {
  try {
    return (await sdcpCall('anywhere.pair.requests', {})).requests;
  } catch {
    return [];
  }
}

export async function confirmPairing(deviceId: string, accept: boolean): Promise<boolean> {
  try {
    await sdcpCall('anywhere.pair.confirm', { deviceId, accept });

    return true;
  } catch (error) {
    failed(error);

    return false;
  }
}

export async function devices(): Promise<AnywhereDevice[]> {
  try {
    return (await sdcpCall('anywhere.devices.list', {})).devices;
  } catch {
    return [];
  }
}

export async function revokeDevice(deviceId: string): Promise<boolean> {
  try {
    await sdcpCall('anywhere.devices.revoke', { deviceId });
    toast(strings.anywhere.devices.revoked_toast);

    return true;
  } catch (error) {
    failed(error);

    return false;
  }
}

export async function resetAnywhere(): Promise<boolean> {
  try {
    await sdcpCall('anywhere.reset', {});
    toast(strings.anywhere.danger.resetDone);

    return true;
  } catch (error) {
    failed(error);

    return false;
  }
}
