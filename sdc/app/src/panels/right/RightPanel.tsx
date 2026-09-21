import {
  ChartColumn,
  CircleCheckBig,
  Clock,
  Eye,
  SquareTerminal,
  Swords,
  type LucideIcon,
} from 'lucide-react';
import { useRef, type ComponentType, type PointerEvent as ReactPointerEvent } from 'react';

import { strings } from '../../strings';
import { useLayoutStore } from '../../store/layout';
import {
  PANEL_TABS,
  consoleErrorCount,
  tabForSession,
  useRightPanelStore,
  type PanelTabDefinition,
  type PanelTabId,
} from '../../store/rightPanel';
import { useSessionsStore } from '../../store/sessions';
import { useAppStore } from '../../store/store';
import { AnalyticsTab } from './AnalyticsTab';
import { ConsoleTab } from './ConsoleTab';
import { DuelTab } from './DuelTab';
import { PreviewTab } from './PreviewTab';
import { TimeMachineTab } from './TimeMachineTab';
import { VerifyTab } from './VerifyTab';

/**
 * The right panel region - spec sections 7.7 to 7.12.
 *
 * Six tabs in a fixed order (Preview, Console, Time Machine, Duel, Verify, Analytics), each with its
 * own view, plus a drag handle on the left edge that resizes the column between 320px and 760px.
 *
 * Three details are worth pointing at:
 *
 *   Sticky per session   which tab is showing is remembered per chat (`tabForSession`), so glancing
 *                        at the Console in one session does not follow you into the next.
 *   Console badge        the error count, read from the same list the Console tab renders.
 *   Simple mode          hides Duel entirely. It is the one place in the UI where the topbar's mode
 *                        switch changes what exists, which is why the tab list is filtered rather
 *                        than rendered straight from the constant.
 *
 * The width lives in the store (so a layout can be restored later) but is applied as
 * `--rightpanel-w` on `#workspace` by src/App.tsx, because this column is the shell grid's third
 * track - the panel cannot set its own width without fighting the grid. The drag handler therefore
 * measures the panel's right edge and writes a width, not a transform.
 */
const TAB_ICON: Record<PanelTabDefinition['icon'], LucideIcon> = {
  eye: Eye,
  terminal: SquareTerminal,
  clock: Clock,
  swords: Swords,
  check: CircleCheckBig,
  chart: ChartColumn,
};

const VIEWS: Record<PanelTabId, ComponentType> = {
  preview: PreviewTab,
  console: ConsoleTab,
  timemachine: TimeMachineTab,
  duel: DuelTab,
  verify: VerifyTab,
  analytics: AnalyticsTab,
};

export function RightPanel() {
  const mode = useLayoutStore((state) => state.mode);
  const showRight = useLayoutStore((state) => state.showRight);
  const rightPanel = useRightPanelStore();
  const { activeTab: activeSessionId, openTabs } = useSessionsStore();
  const panelRef = useRef<HTMLElement | null>(null);

  const sessionId = activeSessionId ?? openTabs[0] ?? null;
  const current = tabForSession(rightPanel, sessionId);
  /* The Console's list is folded from `ConsoleError` events, so the badge and the tab agree. */
  const errorCount = consoleErrorCount(useAppStore((state) => state.console));

  /* Simple mode drops Duel (spec section 7.10); every other tab is always available. */
  const tabs = mode === 'simple' ? PANEL_TABS.filter((tab) => tab.id !== 'duel') : PANEL_TABS;

  /**
   * Drag the left edge. The panel's right edge is fixed - it is the window's, minus nothing - so the
   * width is simply the distance from the pointer to it, clamped by the store.
   */
  const startResize = (event: ReactPointerEvent<HTMLDivElement>): void => {
    const rect = panelRef.current?.getBoundingClientRect();

    if (!rect) {
      return;
    }

    event.preventDefault();

    const right = rect.right;

    const handleMove = (moveEvent: PointerEvent): void => rightPanel.setWidth(right - moveEvent.clientX);

    const stop = (): void => {
      document.removeEventListener('pointermove', handleMove);
      document.removeEventListener('pointerup', stop);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };

    document.addEventListener('pointermove', handleMove);
    document.addEventListener('pointerup', stop);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  };

  return (
    <aside className="rightpanel relative" id="rightpanel" ref={panelRef}>
      {/* Left-edge drag handle (spec section 7.2: the panel folds to 320px, never narrower). */}
      <div
        className="absolute inset-y-0 left-0 z-[5] w-[4px] cursor-col-resize hover:bg-accent-subtle"
        role="separator"
        aria-orientation="vertical"
        aria-label={strings.rightPanel.resize}
        onPointerDown={startResize}
      />

      {/*
        The tabs, as a real `tablist` (0.7.10). They were six plain buttons carrying `aria-selected`, which axe
        reports as a *critical* `aria-allowed-attr`: `aria-selected` is not allowed on a bare button, so a
        screen reader got a tab whose state it was not permitted to announce. `role="tab"` is what makes the
        attribute legal, `aria-controls` names the panel it switches, and the panels below carry
        `role="tabpanel"` + `aria-labelledby` back to the tab.
      */}
      <div
        className="panel-tabs flex shrink-0 items-center gap-[2px] overflow-x-auto border-b border-border-subtle bg-bg-base px-[8px] py-[6px] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        role="tablist"
        aria-label={strings.rightPanel.tabsLabel}
      >
        {tabs.map((tab) => {
          const Icon = TAB_ICON[tab.icon];
          const selected = tab.id === current;

          return (
            <button
              key={tab.id}
              type="button"
              id={`panel-tab-${tab.id}`}
              role="tab"
              data-panel={tab.id}
              aria-selected={selected}
              aria-controls={`panel-view-${tab.id}`}
              tabIndex={selected ? 0 : -1}
              className={
                'panel-tab relative flex items-center gap-[5px] whitespace-nowrap rounded-md px-[9px] py-[6px] text-[11.5px] transition-all duration-fast ease-ease ' +
                (selected
                  ? 'active bg-bg-raised text-text-primary shadow-[inset_0_0_0_1px_var(--border-subtle)]'
                  : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary')
              }
              onClick={() => {
                rightPanel.setActiveTab(tab.id, sessionId);
                /* <=1200px the panel is off until something asks for it (spec section 7.2). */
                showRight();
              }}
            >
              <Icon size={12} aria-hidden="true" />
              {strings.rightPanel.tabs[tab.id]}
              {tab.id === 'console' && errorCount > 0 ? (
                <span className="badge h-[15px] min-w-[15px] rounded-full bg-state-error px-[5px] text-center text-[9.5px] font-bold leading-[15px] text-text-on-accent">
                  {errorCount}
                </span>
              ) : null}
            </button>
          );
        })}
      </div>

      <div className="panel-content relative flex-1 overflow-hidden">
        {tabs.map((tab) => {
          const View = VIEWS[tab.id];

          return (
            <div
              key={tab.id}
              id={`panel-view-${tab.id}`}
              role="tabpanel"
              aria-labelledby={`panel-tab-${tab.id}`}
              data-panel-view={tab.id}
              className={
                'panel-view absolute inset-0 flex-col overflow-y-auto ' +
                (tab.id === current ? 'active flex' : 'hidden')
              }
            >
              <View />
            </div>
          );
        })}
      </div>
    </aside>
  );
}
