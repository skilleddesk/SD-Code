/**
 * The sidebar module (spec section 7.3): the host-grouped session tree.
 *
 * `Sidebar` is the region element. `HostGroup` and `SessionRow` are exported because the sidebar is
 * not the only place a session is drawn - the tab strip's chip needs the same relative time, and
 * the host switcher popover needs the same status dot - and having one implementation of each row
 * is what keeps those three in step.
 */
export { Sidebar } from './Sidebar';
export { HostGroup } from './HostGroup';
export { SessionRow } from './SessionRow';
