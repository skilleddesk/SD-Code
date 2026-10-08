import { Loader } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { strings } from '../../strings';
import { look } from './draftLook';
import type { DraftData } from './types';

/**
 * The tool call the model is still writing (0.14.2).
 *
 * Measured before this card existed: a turn that wrote a 150-line file showed its `Read` card at 6 s and
 * then nothing for 31 s, until the `Write` card arrived already finished. The model had been writing the
 * file the whole time. This card is that time made visible: which file, how much so far, and the newest
 * lines as they arrive - the way you would watch a colleague type. It goes away when the real card opens.
 */
export function DraftCard({ draft }: { draft: DraftData }) {
  const { icon: Icon, verb } = look(draft.name);
  const body = useRef<HTMLPreElement>(null);
  const lines = draft.preview === '' ? 0 : draft.preview.split('\n').length;

  /* The newest line is the one being written: keep it in view as the preview grows. */
  useEffect(() => {
    const element = body.current;

    if (element !== null) {
      element.scrollTop = element.scrollHeight;
    }
  }, [draft.preview]);

  return (
    <div
      className="draft-card mb-[10px] overflow-hidden rounded-md border border-accent/40 bg-bg-raised shadow-sm"
      data-draft={draft.name}
      role="status"
      aria-live="off"
    >
      <div className="flex min-w-0 items-center gap-[8px] px-[10px] py-[7px] text-[12px]">
        <span className="flex h-[20px] w-[20px] shrink-0 items-center justify-center rounded-sm bg-accent-subtle text-accent">
          <Icon size={12} aria-hidden="true" />
        </span>
        <span className="shrink-0 font-mono font-semibold text-text-primary">{draft.name}</span>
        <span className="shrink-0 text-accent">{verb}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-text-secondary" title={draft.target}>
          {draft.target}
        </span>
        <span className="shrink-0 rounded-full bg-accent-subtle px-[7px] py-[1px] font-mono text-[10.5px] tabular-nums text-accent">
          {draft.chars > 0 ? strings.turns.draft.size(draft.chars) : '…'}
          <Elapsed since={draft.since} />
        </span>
        <Loader size={12} aria-hidden="true" className="shrink-0 animate-spin text-accent motion-reduce:animate-none" />
      </div>

      {lines === 0 ? null : (
        <pre
          ref={body}
          className="max-h-[190px] overflow-hidden border-t border-border-subtle bg-bg-input px-[12px] py-[8px] font-mono text-[11.5px] leading-[1.55] text-text-secondary"
          data-draft-preview
        >
          {draft.preview}
          <span className="ml-[1px] animate-pulse text-accent motion-reduce:animate-none" aria-hidden="true">
            ▍
          </span>
        </pre>
      )}
    </div>
  );
}

function Elapsed({ since }: { since: string }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 500);

    return () => window.clearInterval(timer);
  }, []);

  const started = Date.parse(since);

  if (!Number.isFinite(started) || now - started < 1000) {
    return null;
  }

  return <span>{` · ${Math.round((now - started) / 1000)}s`}</span>;
}
