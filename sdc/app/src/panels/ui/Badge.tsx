import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

/**
 * A small pill for a fact about a row (0.7.1): `live` / `cached` / `bundled`, `connected`, `in use`.
 *
 * Every one of those used to be hand-rolled at the call site, which is why the dialog in the screenshot
 * had three different-looking pills - a bordered one for the source, a plain word for the tier, and a
 * button that said `In use` when it was not really a button. One component, four tones, and the meaning
 * is carried by the *word* first and the tone second (spec section 8.6: colour never carries meaning on
 * its own).
 */
export type BadgeTone = 'neutral' | 'muted' | 'accent' | 'success' | 'warning';

const TONES: Record<BadgeTone, string> = {
  neutral: 'border-border-subtle bg-bg-raised text-text-secondary',
  muted: 'border-transparent bg-bg-hover text-text-muted',
  accent: 'border-transparent bg-accent-subtle text-accent',
  success: 'border-transparent bg-green-subtle text-state-success',
  warning: 'border-transparent bg-orange-subtle text-state-waiting',
};

export interface BadgeProps {
  tone?: BadgeTone;
  icon?: LucideIcon;
  title?: string;
  children: ReactNode;
}

export function Badge({ tone = 'neutral', icon: Icon, title, children }: BadgeProps) {
  return (
    <span
      title={title}
      className={
        'inline-flex shrink-0 items-center gap-[4px] whitespace-nowrap rounded-full border px-[8px] py-[2px] ' +
        'font-mono text-[10px] leading-[16px] ' +
        TONES[tone]
      }
    >
      {Icon === undefined ? null : <Icon size={10} aria-hidden="true" />}
      {children}
    </span>
  );
}
