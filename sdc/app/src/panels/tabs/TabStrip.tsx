import { Columns2, Plus } from 'lucide-react';
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';

import { strings } from '../../strings';
import { useLayoutStore } from '../../store/layout';
import { anchorBelow, useOverlayStore } from '../../store/overlays';
import { findSession, useSessionsStore } from '../../store/sessions';
import { toast } from '../../store/toast';
import { IconButton } from '../ui/IconButton';
import { Tab } from './Tab';

/**
 * `.tabstrip` - the 38px strip above the main content (spec section 7.4).
 *
 *   `.tabstrip-scroll-wrap`  the scrolling half, with a 24px fade on its right edge while the tabs
 *                            overflow, so a half-visible tab reads as "there is more" rather than
 *                            "this tab is broken"
 *   `.tabstrip-actions`      the two fixed buttons: `#splitBtn` (columns-2, lit while split view is
 *                            on) and `#newTabBtn` (plus, which opens the host picker)
 *
 * Two behaviours are worth the code below:
 *
 *   Overflow       `scrollWidth > clientWidth` is measured rather than guessed, on every tab change
 *                  and on every resize of the strip - a `ResizeObserver` on the wrap, because the
 *                  sidebar collapsing is what usually causes it. The +4px slack is the prototype's:
 *                  without it a strip that is exactly full flickers its fade on and off.
 *   Scroll-into-view  only the active tab is scrolled to, and with `inline: 'nearest'` so a tab that
 *                  is already visible does not move. Opening a session from the sidebar therefore
 *                  drags the strip just far enough to show its new tab.
 *
 * Ctrl+backslash toggles split from anywhere (src/layout/useShellLayout.ts, registered in App.tsx);
 * this button is the mouse's way to the same store field, and both announce themselves with a toast.
 */
/**
 * The right-edge fade, drawn by the scroll wrap's `::after` and switched on by `overflowing`. It is
 * a constant rather than inline markup because the `content` declaration needs a pair of single
 * quotes, which is awkward inside a class list built with string concatenation.
 */
const OVERFLOW_FADE =
  "after:pointer-events-none after:absolute after:inset-y-0 after:right-0 after:w-[24px] after:content-[''] after:[background-image:linear-gradient(90deg,transparent,var(--bg-base))]";

export function TabStrip() {
  const { openTabs, activeTab, hosts, closeTab, openSession } = useSessionsStore();
  const split = useLayoutStore((state) => state.split);
  const toggleSplit = useLayoutStore((state) => state.toggleSplit);
  const openNewChat = useOverlayStore((state) => state.openNewChat);

  const wrapRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [overflowing, setOverflowing] = useState(false);

  /** Measure the strip and pull the active tab into view. */
  const sync = useCallback((): void => {
    const wrap = wrapRef.current;
    const scroll = scrollRef.current;

    if (!wrap || !scroll) {
      return;
    }

    setOverflowing(scroll.scrollWidth > scroll.clientWidth + 4);

    const active = scroll.querySelector<HTMLElement>('.tab.active');
    active?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, []);

  /* Layout effect: the strip must be correct in the frame the tab appears in, not the next one. */
  useLayoutEffect(sync, [sync, openTabs, activeTab]);

  useEffect(() => {
    const wrap = wrapRef.current;

    if (!wrap || typeof ResizeObserver === 'undefined') {
      return;
    }

    const observer = new ResizeObserver(sync);
    observer.observe(wrap);

    return () => observer.disconnect();
  }, [sync]);

  const handleSplit = (): void => {
    toggleSplit();
    toast(split ? strings.tabs.split.off : strings.tabs.split.on);
  };

  return (
    <div className="tabstrip relative flex min-w-0 shrink-0 items-stretch border-b border-border-subtle bg-bg-base h-[38px]">
      <div
        ref={wrapRef}
        className={
          'tabstrip-scroll-wrap relative min-w-0 flex-1 overflow-hidden ' +
          (overflowing ? OVERFLOW_FADE : '')
        }
        id="tabScrollWrap"
      >
        <div
          ref={scrollRef}
          className="tabstrip-scroll flex h-full items-stretch overflow-x-auto [scrollbar-width:none] [scroll-behavior:smooth] [&::-webkit-scrollbar]:hidden"
          id="tabstripScroll"
        >
          {openTabs.map((id) => {
            const ref = findSession(hosts, id);

            if (!ref) {
              return null;
            }

            return (
              <Tab
                key={id}
                id={id}
                title={ref.session.title}
                hostName={ref.host.name}
                state={ref.session.state}
                active={id === activeTab}
                onOpen={() => openSession(id)}
                onClose={() => closeTab(id)}
              />
            );
          })}
        </div>
      </div>

      <div className="tabstrip-actions flex shrink-0 items-center gap-[2px] border-l border-border-subtle px-[6px]">
        <IconButton
          id="splitBtn"
          icon={Columns2}
          label={strings.tabs.split.title}
          active={split}
          size={26}
          iconSize={14}
          onClick={handleSplit}
        />
        <IconButton
          id="newTabBtn"
          icon={Plus}
          label={strings.tabs.newTab.title}
          size={26}
          iconSize={14}
          onClick={(event) => openNewChat(anchorBelow(event.currentTarget))}
        />
      </div>
    </div>
  );
}
