/**
 * `app/src/panels/topbar/IconButton.tsx` - spec section 7.1 lists the topbar's four icon buttons,
 * and the step that builds them names this file. The component is shared with the tab strip, the
 * prompt toolbar and the right panel's Preview toolbar, so the implementation lives one level up in
 * `src/panels/ui/IconButton.tsx` and this module re-exports it: importers can keep saying
 * `./IconButton` from inside the topbar, and every other panel can reach the same button.
 */
export { IconButton } from '../ui/IconButton';
export type { IconButtonProps } from '../ui/IconButton';
