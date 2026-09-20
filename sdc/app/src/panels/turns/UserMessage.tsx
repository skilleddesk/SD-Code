import { Image as ImageIcon } from 'lucide-react';

import type { AttachmentData, UserMessageData } from './types';

/**
 * `.user-msg` - what you asked for (spec section 7.5).
 *
 * Three parts, in order:
 *
 *   .who    `You · 14:02`, uppercase with .06em of letter-spacing, followed by a hairline that
 *           fills the rest of the row - the `::after` trick is not available here, so the line is
 *           a sibling that flexes, which is the same picture and keeps the text selectable.
 *   .body   14.5px at 1.6 line-height: the largest text in the app, because it is the one thing on
 *           screen the user wrote.
 *   .meta   the attachment chips: a gradient thumbnail, `screenshot.png` behind an image icon, and
 *           `@src/auth.ts` in the accent tint.
 */
export interface UserMessageProps {
  message: UserMessageData;
}

/**
 * `.chip` - the shared pill: raised surface, mono 11px, hairline.
 *
 * The two tones are separate constants rather than one base plus overrides, because Tailwind emits
 * same-property utilities in its own order and a class list cannot win an argument with it: an
 * `accent` chip written as `CHIP + ' bg-accent-subtle text-accent'` keeps `--bg-raised` and
 * `--text-secondary`, which is exactly what the first measurement of this step showed.
 */
const CHIP_BASE =
  'chip inline-flex items-center gap-[5px] rounded-md border px-[8px] py-[3px] font-mono text-[11px]';

const CHIP = CHIP_BASE + ' border-border-subtle bg-bg-raised text-text-secondary';

/** `.chip.accent` - the `@src/auth.ts` reference: an accent tint with an accent border. */
const CHIP_ACCENT = CHIP_BASE + ' accent border-[rgba(91,156,255,.3)] bg-accent-subtle text-accent';

function Attachment({ attachment }: { attachment: AttachmentData }) {
  if (attachment.kind === 'thumb') {
    return (
      <span className={CHIP + ' thumb p-[2px]'}>
        <span className="thumb-img block h-[30px] w-[44px] rounded-[3px] shadow-[inset_0_0_0_1px_rgba(255,255,255,.05)] [background-image:var(--grad-thumb)]" />
      </span>
    );
  }

  if (attachment.kind === 'image') {
    return (
      <span className={CHIP}>
        <ImageIcon size={11} aria-hidden="true" />
        {attachment.label}
      </span>
    );
  }

  return (
    <span className={CHIP_ACCENT}>{attachment.label}</span>
  );
}

export function UserMessage({ message }: UserMessageProps) {
  return (
    <div className="user-msg mb-[14px]">
      <div className="who mb-[8px] flex items-center gap-[10px] text-[10.5px] font-semibold uppercase tracking-[.06em] text-text-muted">
        {message.who}
        <span className="h-px flex-1 bg-border-subtle" aria-hidden="true" />
      </div>

      <div className="body text-[14.5px] leading-[1.6] text-text-primary">{message.body}</div>

      {message.attachments.length === 0 ? null : (
        <div className="meta mt-[10px] flex flex-wrap items-center gap-[6px]">
          {message.attachments.map((attachment, index) => (
            <Attachment key={`${attachment.kind}-${index}`} attachment={attachment} />
          ))}
        </div>
      )}
    </div>
  );
}
