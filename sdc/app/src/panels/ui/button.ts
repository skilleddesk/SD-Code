/**
 * The shared button classes (spec section 8.3: "button sm 24px / md 28px / lg 34px").
 *
 * Buttons are written with utilities rather than as a component, because there is nothing to hold
 * in state: the difference between the seven variants below is entirely which classes they carry.
 * The four surfaces that need one - the error card, the Preview footer, the Console footer and the
 * Verify footer - import these strings instead of re-typing them, so "primary" means one thing.
 *
 * The heights match the spec's three sizes: `BTN_SM` 24px for a row that also carries a 16px icon,
 * `BTN` 28px for the default, and the full-width footers use `BTN_BLOCK`.
 */

/** `.btn` - raised surface, hairline, 28px. */
export const BTN =
  'btn inline-flex h-[28px] items-center gap-[6px] whitespace-nowrap rounded-md border px-[11px] py-[5px] ' +
  'text-[11.5px] font-medium transition-all duration-fast ease-ease active:scale-[.97]';

/** `.btn.sm` - 24px, for the Duel tab's two foot buttons. */
export const BTN_SM =
  'btn sm inline-flex h-[24px] items-center gap-[6px] whitespace-nowrap rounded-md border px-[9px] py-[3px] ' +
  'text-[10.5px] font-medium transition-all duration-fast ease-ease active:scale-[.97]';

/** Default tone: raised, hairline on --border-default. The prototype gives this no extra class. */
export const BTN_SECONDARY =
  'border-border-default bg-bg-raised text-text-primary hover:border-border-strong hover:bg-bg-hover';

/**
 * `.btn.primary` - the accent fill; one per surface. The `primary` class is kept even though the
 * tint comes from utilities, because the prototype's markup names the variant and a DOM diff
 * against design/ui-prototype.html should find the same words.
 */
export const BTN_PRIMARY =
  'primary border-accent bg-accent text-text-on-accent hover:border-accent-hover hover:bg-accent-hover';

/** Full-width variant, used by the three panel footers. */
export const BTN_BLOCK = 'w-full justify-center';

