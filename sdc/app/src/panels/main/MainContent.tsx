import { useEffect } from 'react';

import { useLayoutStore } from '../../store/layout';
import { findSession, useSessionsStore } from '../../store/sessions';
import { EmptyState } from './EmptyState';
import { Pane } from './Pane';

/**
 * `.main-content#mainContent` - the flexible middle of the main column (spec sections 7.4, 9.15).
 *
 * Three states, and no others:
 *
 *   no tab open   the empty state of spec section 7.13 fills the area
 *   one chat      a single pane, edge to edge
 *   split         two panes side by side with a hairline between them
 *
 * The second pane's session is `splitSecondary` in the session store, and it is re-picked when the
 * split turns on (`ensureSplitSecondary`, "the first tab that is not the active one" - spec section
 * 9.15) or when the pane it pointed at is closed. The effect below is that re-pick: the store cannot
 * do it by itself, because it is the split flag - which lives in the layout store next to the shell
 * geometry - that says whether the pane is needed at all.
 *
 * Split and the sidebar/right panel are independent: the shell grid still has three columns, and the
 * two panes share the middle one.
 */
export function MainContent() {
  const split = useLayoutStore((state) => state.split);
  const { hosts, activeTab, splitSecondary, ensureSplitSecondary } = useSessionsStore();

  const primary = findSession(hosts, activeTab);
  const secondary = findSession(hosts, splitSecondary);

  useEffect(() => {
    if (split) {
      ensureSplitSecondary();
    }
  }, [split, activeTab, splitSecondary, ensureSplitSecondary]);

  return (
    <div className="main-content relative flex flex-1 overflow-hidden" id="mainContent">
      {primary === null ? (
        <EmptyState />
      ) : split && secondary !== null ? (
        <>
          <Pane session={primary.session} host={primary.host} showHeader />
          <Pane session={secondary.session} host={secondary.host} showHeader />
        </>
      ) : (
        <Pane session={primary.session} host={primary.host} showHeader={false} />
      )}
    </div>
  );
}
