/**
 * The five shell regions - spec section 7.0 - and the document tree they are arranged in.
 *
 * This file is the module boundary, not the implementation: each region belongs to the panel that
 * fills it, and this step replaced the placeholder boxes of STEP 3 with those panels. The trees are
 * one-directional, so there is no cycle:
 *
 *   Topbar      src/panels/topbar       brand, host pill, mode switch, palette, four icon buttons
 *   Sidebar     src/panels/sidebar      host-grouped session tree
 *   MainArea    src/panels/main         degraded banner, tab strip, empty state / one or two panes
 *   RightPanel  src/panels/right        six tabs
 *   StatusBar   src/panels/statusbar    seven segments
 *
 * The region *elements* - `div.topbar`, `aside.sidebar#sidebar`, `main.main`,
 * `aside.rightpanel#rightpanel`, `div.statusbar` - are still the prototype's, and their geometry is
 * still src/layout/Shell.css: 46px / 1fr / 30px down the window, 280px / 1fr / 400px across the
 * workspace, the 1200px and 900px breakpoints, and the `no-sidebar`, `no-right`, `show-right`,
 * `mobile-sidebar-open` and `mobile-backdrop` classes that `workspaceClassName()` produces. Anything
 * a reader wants to know about how the shell reflows is in that file; anything about what a region
 * contains is in the panel.
 *
 * `import './Shell.css'` stays here so the geometry arrives with the shell whether or not the panels
 * are mounted.
 */

import './Shell.css';

export { Topbar } from '../panels/topbar';
export { Sidebar } from '../panels/sidebar';
export { MainArea } from '../panels/main';
export { RightPanel } from '../panels/right';
export { StatusBar } from '../panels/statusbar';

