import { Image as ImageIcon, Languages } from 'lucide-react';

import { strings } from '../../strings';

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
const CHIP_ACCENT = CHIP_BASE + ' accent border-accent/30 bg-accent-subtle text-accent';

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
  /* 0.18: the person's words in a bubble on the right - the one thing on screen they wrote - and the
     agent's work on the left under it, the way every conversation reads. */
  return (
    <div className="user-msg mb-[16px] flex flex-col items-end">
      <div className="who sr-only">{message.who}</div>

      <div className="body max-w-[86%] whitespace-pre-wrap break-words rounded-[18px] rounded-br-[6px] border border-accent/20 px-[15px] py-[10px] text-[14px] leading-[1.6] text-text-primary shadow-sm [background-image:var(--grad-brand-soft)]">
        {message.body}
      </div>

      {message.reading === undefined ? null : (
        <div
          className="reading mt-[6px] inline-flex items-center gap-[5px] rounded-full border border-border-subtle bg-bg-raised px-[8px] py-[2px] text-[10.5px] text-text-muted"
          title={strings.turns.readingTitle}
          data-reading={message.reading.label}
        >
          <Languages size={11} aria-hidden="true" className="text-accent" />
          {strings.turns.reading(message.reading.label, message.reading.reply)}
        </div>
      )}

      {message.attachments.length === 0 ? null : (
        <div className="meta mt-[8px] flex flex-wrap items-center justify-end gap-[6px]">
          {message.attachments.map((attachment, index) => (
            <Attachment key={`${attachment.kind}-${index}`} attachment={attachment} />
          ))}
        </div>
      )}
    </div>
  );
}
