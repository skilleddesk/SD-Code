import type { TelemetryView } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';

/** Opt-in telemetry (Phase 4.2): the window only asks the daemon; the choice, the counts and the endpoint live there. */
export async function loadTelemetry(): Promise<TelemetryView | null> {
  try {
    return await sdcpCall('telemetry.status', {});
  } catch {
    return null;
  }
}

export async function setTelemetry(optIn: boolean, endpoint: string): Promise<TelemetryView | null> {
  try {
    return await sdcpCall('telemetry.set', { optIn, ...(endpoint.trim() === '' ? {} : { endpoint: endpoint.trim() }) });
  } catch {
    return null;
  }
}

/** The person's own press. Answers a sentence for the card: what was sent, or why not. */
export async function sendTelemetry(): Promise<string> {
  try {
    const answer = await sdcpCall('telemetry.send', {});

    return strings.telemetry.sent(answer.sent.counts.length, answer.status);
  } catch (reason) {
    return isSdcpError(reason) ? reason.message : strings.telemetry.sendFailed;
  }
}

export async function clearTelemetry(): Promise<void> {
  try {
    await sdcpCall('telemetry.clear', {});
  } catch {
    /* nothing to clear */
  }
}
