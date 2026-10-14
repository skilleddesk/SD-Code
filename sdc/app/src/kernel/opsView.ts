/**
 * The Workbench's three readings of a daemon value, kept out of the component file: the react-refresh rule
 * wants a `.tsx` to export components and nothing else, and each of these is worth a test of its own.
 */
import type { OpsAutomation } from '../store/kernelIntents';
import { strings } from '../strings';

const k = strings.kernel.ops;

/** How long ago, in words. A run that has not happened is "never", not "56 years ago". */
export function ago(at: number | null | undefined, now = Math.floor(Date.now() / 1000)): string {
  if (!at) return k.inbox.never;

  const minutes = Math.max(0, Math.round((now - at) / 60));

  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;

  const hours = Math.round(minutes / 60);

  return hours < 24 ? `${hours} h ago` : `${Math.round(hours / 24)} d ago`;
}

/** How an agent's state reads, and how loudly - the word carries it, the tone only agrees with it. */
export function stateOf(
  state: string,
): { label: string; tone: 'neutral' | 'muted' | 'accent' | 'success' | 'warning' } {
  switch (state) {
    case 'running':
      return { label: k.agents.running, tone: 'accent' };
    case 'done':
      return { label: k.agents.done, tone: 'success' };
    case 'failed':
      return { label: k.agents.failed, tone: 'warning' };
    default:
      return { label: k.agents.dead, tone: 'muted' };
  }
}

/** An automation's clock, as the file expressed it. */
export function whenOf(automation: Pick<OpsAutomation, 'every' | 'at'>): string {
  if (automation.every) return k.inbox.every(automation.every);

  return automation.at ? k.inbox.at(automation.at) : k.inbox.when;
}

