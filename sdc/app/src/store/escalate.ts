import { create } from 'zustand';

import type { EscalateAnswer } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';
import { toast } from './toast';

/**
 * The manual Escalate (Phase 3.5). Two calls and a click between them: `preview` asks the daemon what *would* be sent and what it may
 * cost (nothing leaves the computer); `send` is the click. Nothing here runs by itself.
 */
interface EscalateState {
  sessionId: string | null;
  preview: EscalateAnswer | null;
  busy: boolean;
  error: string | null;
}

export const useEscalate = create<EscalateState>()(() => ({ sessionId: null, preview: null, busy: false, error: null }));

const message = (reason: unknown): string => (isSdcpError(reason) ? reason.message : strings.escalate.failed);

/** Asks what sending this chat's failed step to the next chain entry would send and cost. */
export async function requestEscalate(sessionId: string): Promise<void> {
  useEscalate.setState({ sessionId, preview: null, busy: true, error: null });

  try {
    const preview = await sdcpCall('rotation.escalate', { sessionId, confirm: false });

    useEscalate.setState({ preview, busy: false });
  } catch (reason) {
    useEscalate.setState({ busy: false, error: message(reason) });
  }
}

/** The click: sends the previewed step. */
export async function sendEscalate(autonomy?: string): Promise<void> {
  const { sessionId, preview } = useEscalate.getState();

  if (sessionId === null || preview === null) {
    return;
  }

  useEscalate.setState({ busy: true, error: null });

  try {
    const sent = await sdcpCall('rotation.escalate', { sessionId, confirm: true, ...(autonomy === undefined ? {} : { autonomy }) });

    toast(strings.escalate.started(sent.model));
    cancelEscalate();
  } catch (reason) {
    useEscalate.setState({ busy: false, error: message(reason) });
  }
}

export function cancelEscalate(): void {
  useEscalate.setState({ sessionId: null, preview: null, busy: false, error: null });
}
