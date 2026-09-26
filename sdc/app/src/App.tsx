import type { CSSProperties } from 'react';
import { useEffect } from 'react';

import { MainArea, RightPanel, Sidebar, StatusBar, Topbar } from './layout/Shell';
import { useResponsiveShell } from './layout/useShellLayout';
import { useKeys } from './hooks/useKeys';
import { AddHost } from './modals/AddHost';
import { Connect } from './modals/Connect';
import { HostSwitcherPopover } from './modals/HostSwitcherPopover';
import { Permission } from './modals/Permission';
import { ProviderHub } from './modals/ProviderHub';
import { NewProject } from './modals/NewProject';
import { RemoteFolder } from './modals/RemoteFolder';
import { Settings } from './modals/Settings';
import { KeymapReference } from './overlays/KeymapReference';
import { NewChatPopover } from './overlays/NewChatPopover';
import { Palette } from './overlays/Palette';
import { SearchOverlay } from './overlays/SearchOverlay';
import { Toast } from './overlays/Toast';
import { connectDaemon, watchBackground, watchDaemon } from './store/intents';
import { useLayoutStore, workspaceClassName } from './store/layout';
import { usePrefixHint } from './store/prefs';
import { useRightPanelStore } from './store/rightPanel';

/**
 * Composition root of the SDC shell (spec sections 7.0 and 7.2).
 *
 * The document tree is fixed by spec section 7.0 and is the prototype's:
 *
 *   #app                      grid 46px / 1fr / 30px
 *   |-- .topbar               brand, host pill, mode switch, palette, icon buttons   (7.1)
 *   |-- .workspace#workspace  grid 280px / 1fr / 400px
 *   |   |-- .sidebar#sidebar  host-grouped session tree                              (7.3)
 *   |   |-- .main             tab strip, turn stream, prompt area                    (7.4-7.6)
 *   |   `-- .rightpanel       six tabs                                               (7.7-7.12)
 *   `-- .statusbar            seven segments                                         (7.15)
 *
 * Everything else is mounted *outside* `#app`, at the body level, because each one has to escape the
 * region it was triggered from:
 *
 *   AddHost, ProviderHub, Settings, Permission, KeymapReference   centred dialogs (9.10-9.13, 9.1)
 *   Palette, SearchOverlay                                        top-aligned overlays (9.2, 9.4)
 *   NewChatPopover, HostSwitcherPopover                           positioned at their trigger (9.5, 7.1)
 *   Toast                                                         above everything, always (9.14)
 *
 * Three hooks wire the app to the outside world, and all three are called exactly once, here:
 * `useResponsiveShell()` (the window width), `useKeys()` (the registry's single keyboard listener)
 * and `usePrefixHint()` (the one startup toast of spec section 9.14).
 *
 * `--rightpanel-w` is written here rather than by the panel itself: the right column is a track of
 * the workspace grid, so the only place that can resize it is the grid container.
 */
export function App() {
  useResponsiveShell();
  useKeys();
  usePrefixHint();

  /*
   * One handshake with the daemon, once per window (spec section 5.4). It is what makes an installed
   * app work on a double-click: `host.status` is the first SDCP call, and a refused connection is
   * what tells the Tauri bridge to start `sdcd` and wait for its port. It also folds this machine's
   * real status into the store, which the seed alone cannot do.
   */
  useEffect(() => {
    void connectDaemon();
  }, []);

  /*
   * The heartbeat - the question that keeps being asked (spec sections 3.1 and 5.4).
   *
   * `connectDaemon()` is one handshake. Without a heartbeat after it, a daemon that dies mid-session
   * is invisible: `sdcd` is a child process, and a child that has gone does not knock. The return
   * value is the stop function, so this effect owns the timer for exactly as long as the window is
   * open (see `store/daemon.ts` for why the answer is presentation and not an event).
   */
  useEffect(() => watchDaemon(), []);

  /*
   * The Terminal's background tail (0.7.13). It is a second timer rather than a hook in the tab, and
   * deliberately: a process keeps printing while you look at the Preview or another chat, so its output
   * has to keep filling in - a tail that only advances while it is on screen is a tail that lies about
   * what it collected. The poll reads nothing and returns immediately when there is no process.
   */
  useEffect(() => watchBackground(), []);

  const layout = useLayoutStore();
  const rightPanelWidth = useRightPanelStore((state) => state.width);

  const shellStyle = { '--rightpanel-w': `${rightPanelWidth}px` } as CSSProperties;

  return (
    <>
      <div id="app">
        <Topbar />

        <div id="workspace" className={workspaceClassName(layout)} style={shellStyle}>
          <Sidebar />
          <MainArea />
          <RightPanel />
        </div>

        <StatusBar />
      </div>

      <NewChatPopover />
      <HostSwitcherPopover />

      <Palette />
      <SearchOverlay />
      <KeymapReference />

      <ProviderHub />
      <Settings />
      <AddHost />
      <RemoteFolder />
      <NewProject />
      <Permission />
      <Connect />

      <Toast />
    </>
  );
}

