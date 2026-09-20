/**
 * The main module (spec sections 7.4-7.6, 7.13, 9.15).
 *
 * `MainArea` is the region element the shell re-exports; `MainContent` is the part under the tab
 * strip, which is what a reader of the spec's §7.4 diagram will be looking for. `Pane` and
 * `EmptyState` are exported because the Duel tab and the sessions-overview screen will both want the
 * same two shapes.
 */
export { MainArea } from './MainArea';
export { MainContent } from './MainContent';
export { Pane } from './Pane';
export { EmptyState } from './EmptyState';
export { DegradedBanner } from './DegradedBanner';
