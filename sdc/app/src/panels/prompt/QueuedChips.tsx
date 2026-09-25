import { X } from 'lucide-react';

import { strings } from '../../strings';
import { useModelStore } from '../../store/model';

/**
 * `.queued-chips` - prompts waiting behind the running turn (spec sections 7.6 and 9.7).
 *
 * Queued steering is what stops a running turn from being an argument: you can add up to three more
 * instructions while the engine works, and they go out together when it finishes. Each chip is one
 * of those, accent-tinted, with an `x` that takes it back off the queue - which is the whole
 * interaction, because the sending happens on the turn boundary.
 *
 * The store caps the list at `MAX_QUEUED_PROMPTS`, so this component only ever renders what the
 * store allowed in.
 */
export function QueuedChips({ sessionId }: { sessionId?: string }) {
  const all = useModelStore((state) => state.queued);
  const dequeue = useModelStore((state) => state.dequeue);
  const queued = all.filter((item) => item.sessionId === sessionId).map((item) => item.prompt);

  if (queued.length === 0) {
    return null;
  }

  return (
    <div className="queued-chips mb-[8px] flex flex-wrap gap-[6px]">
      {queued.map((prompt) => (
        <span
          key={prompt}
          className="queued-chip flex items-center gap-[6px] rounded-md border border-[rgba(91,156,255,.3)] bg-accent-subtle px-[8px] py-[4px] text-[11px] text-accent"
        >
          {prompt}
          <button
            type="button"
            className="grid h-[14px] w-[14px] place-items-center rounded-sm text-accent hover:bg-[rgba(91,156,255,.2)] hover:text-accent-hover"
            title={strings.prompt.queued.remove}
            aria-label={strings.prompt.queued.remove}
            onClick={() => {
              if (sessionId !== undefined) {
                dequeue(sessionId, prompt);
              }
            }}
          >
            <X size={10} aria-hidden="true" />
          </button>
        </span>
      ))}
    </div>
  );
}
