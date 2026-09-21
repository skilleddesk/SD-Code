/**
 * The shared button classes (spec section 8.3: "button sm 24px / md 28px / lg 34px").
 *
 * Buttons are written with utilities rather than as a component, because there is nothing to hold in
 * state: the difference between the variants below is entirely which classes they carry. The surfaces
 * that need one import these strings instead of re-typing them, so `primary` means one thing.
 *
 * **0.7.1 fixed two things a screenshot showed.**
 *
 *  1. **A disabled button looked enabled.** The tones never carried a `disabled:` state, so `Save` on an
 *     empty key field was the same saturated accent as `Save` on a filled one - the report was *"button
 *     gulaw useless"*, and this is why: nothing on screen said whether a press would do anything. Every
 *     base class now dims and stops taking the pointer, and that includes the primary one.
 *  2. **Every button looked the same size and weight**, so a dialog had no hierarchy: the destructive,
 *     the primary and the quiet action were one shape. Tones are named for what they are, and there are
 *     three sizes - `BTN_SM` (24), `BTN` (28), `BTN_LG` (34) - instead of one used everywhere.
 */

/** `.btn` - raised surface, hairline, 28px. The base every tone adds to. */
export const BTN =
  'btn inline-flex h-[28px] shrink-0 items-center justify-center gap-[6px] whitespace-nowrap rounded-md border ' +
  'px-[11px] py-[5px] text-[11.5px] font-medium transition-all duration-fast ease-ease active:scale-[.97] ' +
  'disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100';

/** `.btn.sm` - 24px, for a row's own action. */
export const BTN_SM =
  'btn sm inline-flex h-[24px] shrink-0 items-center justify-center gap-[5px] whitespace-nowrap rounded-md border ' +
  'px-[9px] py-[3px] text-[10.5px] font-medium transition-all duration-fast ease-ease active:scale-[.97] ' +
  'disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100';

/** `.btn.lg` - 34px, for the footer's one decision. */
export const BTN_LG =
  'btn lg inline-flex h-[34px] shrink-0 items-center justify-center gap-[7px] whitespace-nowrap rounded-md border ' +
  'px-[15px] py-[7px] text-[12.5px] font-semibold transition-all duration-fast ease-ease active:scale-[.97] ' +
  'disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100';

/** `sm`'s classes with the `lg` geometry: same base, the size the caller wants. */
export const BTN_SM_LG =
  'btn sm lg inline-flex h-[34px] shrink-0 items-center justify-center gap-[6px] whitespace-nowrap rounded-md border ' +
  'px-[12px] py-[6px] text-[12px] font-medium transition-all duration-fast ease-ease active:scale-[.97] ' +
  'disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100';

/** Default tone: raised, hairline on --border-default. A dialog's "no". */
export const BTN_SECONDARY =
  'border-border-default bg-bg-raised text-text-primary hover:border-border-strong hover:bg-bg-hover';

/** A quiet action: no border, no fill, secondary text. For `Refresh`, `Load models`, `Copy`. */
export const BTN_GHOST =
  'border-transparent bg-transparent text-text-secondary hover:border-border-default hover:bg-bg-hover hover:text-text-primary';

/**
 * `.btn.primary` - the accent fill; one per surface. The `primary` class is kept even though the tint
 * comes from utilities, because the prototype's markup names the variant and a DOM diff against
 * design/ui-prototype.html should find the same words.
 */
export const BTN_PRIMARY =
  'primary border-accent-fill bg-accent-fill text-text-on-accent hover:border-accent-fill-hover hover:bg-accent-fill-hover';

/** Destructive: `Remove this host`, and nothing else. */
export const BTN_DANGER =
  'danger border-red-subtle bg-red-subtle text-state-error hover:border-red hover:bg-red-subtle';

/** Full-width variant, used by the three panel footers. */
export const BTN_BLOCK = 'w-full justify-center';


