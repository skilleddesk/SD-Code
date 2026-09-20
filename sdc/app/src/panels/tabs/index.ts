/**
 * The tab strip module (spec section 7.4). `TabStrip` is the strip; `Tab` is one tab, exported
 * because a second strip may exist later (a split view has one pane header per pane, and a future
 * "tab bar per pane" would reuse this row rather than re-draw it).
 */
export { TabStrip } from './TabStrip';
export { Tab } from './Tab';
export type { TabProps } from './Tab';
