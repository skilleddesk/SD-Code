import { useEffect } from 'react';

import { commandsForSpec, keySpec } from '../commands/registry';

/**
 * The single global keyboard listener - spec section 9.1.
 *
 * One `keydown` handler on `document`, and it does nothing but: normalize the event into a spec,
 * ask the registry what that spec means *right now*, and run the answer. Every shortcut in the app
 * comes through here, so the three rules below hold everywhere at once instead of being re-stated
 * per component:
 *
 *   Focus rule      "typing inside textarea/input does not trigger Ctrl+letter unless documented".
 *                   When a field has focus, only commands marked `inInput` are eligible. That is
 *                   why `Enter` in the prompt area still inserts a newline (only the approval
 *                   dialog's `Enter` is registered, and only while that dialog is up) and why a
 *                   bare `j` in a textarea types a `j`.
 *   `when()`        the registry's predicate is checked at press time, which is how `Esc` can mean
 *                   "deny" while the approval dialog is open and "close the overlay" otherwise.
 *   preventDefault  only when a command actually matched. A key nobody claims is left to the
 *                   browser, so `Ctrl+R`, `F5` and the platform's own shortcuts keep working.
 *
 * The palette and the search overlay have listeners of their own (arrow keys, `Enter`, `Esc` inside
 * their inputs). They call `stopPropagation()`, so this handler never sees those presses - the two
 * never fight over the same keystroke.
 *
 * Call it once, from the composition root.
 */
export function useKeys(): void {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent): void => {
      /* An event already handled by a surface above us was stopped, never re-dispatched. */
      if (event.defaultPrevented) {
        return;
      }

      const spec = keySpec(event);
      const inField = isEditable(document.activeElement);

      const command = commandsForSpec(spec).find(
        (candidate) => candidate.inInput === true || !inField,
      );

      if (!command) {
        return;
      }

      event.preventDefault();
      command.run();
    };

    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, []);
}

/**
 * True when the focused element takes text. `SELECT` counts because a `j` typed into a dropdown
 * should jump its options, not the timeline; `contentEditable` counts because a future rich prompt
 * box would need the same treatment.
 */
export function isEditable(element: Element | null): boolean {
  if (element === null) {
    return false;
  }

  if (element instanceof HTMLElement && element.isContentEditable) {
    return true;
  }

  return ['INPUT', 'TEXTAREA', 'SELECT'].includes(element.tagName);
}
