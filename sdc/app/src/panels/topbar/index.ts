/**
 * The topbar module (spec section 7.1). `Topbar` is the region element; the pieces are exported
 * too, because the topbar is not the only place they appear:
 *
 *   HostPill    the status bar's host item opens the same switcher popover (spec section 7.15)
 *   IconButton  the tab strip, the Preview toolbar and the prompt toolbar all use it
 *   ModeSwitch  belongs to the topbar alone
 */
export { Topbar } from './Topbar';
export { HostPill } from './HostPill';
export { ModeSwitch } from './ModeSwitch';
export { IconButton } from './IconButton';
export type { IconButtonProps } from './IconButton';
