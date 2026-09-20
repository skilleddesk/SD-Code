import type { ReactNode } from 'react';

/**
 * A titled block inside a dialog (0.7.1).
 *
 * The connection dialogs used to be a flat column of controls: an unlabelled input, two buttons on the
 * same line as a footnote, then a list. A screenshot of the API-key dialog is what caused this file -
 * *"koto useless and normal"*, and the honest reading is that nothing on it said what was a section,
 * what was an action, or what belonged to what.
 *
 * A section is the missing unit: a small uppercase name, an optional right-hand action, an optional hint
 * beside the name, and a body under it. Two of them in a dialog read as two jobs; without them the
 * dialog reads as a pile.
 */
export interface SectionProps {
  /** The uppercase name: `API KEY`, `MODELS`, `THE CLI'S OWN OUTPUT`. */
  title: string;
  /** A quiet fact about the section, set in mono: `2 of 3 · cached`. */
  hint?: string;
  /** Right-hand controls that belong to the section as a whole: `Refresh`, `Load models`. */
  action?: ReactNode;
  /** A sentence under the body: where the key is kept, why a list is stale. */
  note?: ReactNode;
  children: ReactNode;
}

export function Section({ title, hint, action, note, children }: SectionProps) {
  return (
    <section className="dialog-section border-b border-border-subtle last:border-b-0">
      <header className="flex items-center gap-[8px] px-[18px] pb-[8px] pt-[14px]">
        <h3 className="text-[10px] font-bold uppercase tracking-[.12em] text-text-muted">{title}</h3>

        {hint === undefined ? null : (
          <span className="truncate font-mono text-[10px] text-text-faint">{hint}</span>
        )}

        {action === undefined ? null : <div className="ml-auto flex items-center gap-[6px]">{action}</div>}
      </header>

      <div className="px-[18px] pb-[14px]">{children}</div>

      {note === undefined ? null : (
        <p className="px-[18px] pb-[14px] text-[11.5px] leading-[1.5] text-text-muted">{note}</p>
      )}
    </section>
  );
}
