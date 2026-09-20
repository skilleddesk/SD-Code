import { create } from 'zustand';

import { DEFAULT_THEME, type Theme } from '../lib/theme';

/**
 * Shell layout store - the single source of truth for which of the shell's three regions are on
 * screen (spec section 7.2). Nothing else decides this: `workspaceClassName()` below is the only
 * writer of the classes in src/layout/Shell.css, and the topbar buttons of STEP 4, the mobile
 * backdrop of STEP 13 and the keyboard hook all call the actions here.
 *
 * The four fields are the ones spec section 7.2 needs, nothing more:
 *
 *   sidebar            'visible' is the 280px column, 'collapsed' is the `no-sidebar` class.
 *   right              'visible' carries `show-right`, 'hidden' carries `no-right`. The two classes
 *                      are mutually exclusive, which is what makes them one field.
 *   split              Split view (spec section 9.15). It belongs to the main area - the two panes
 *                      live inside `.main-content` - so it does *not* touch the shell grid, and
 *                      nothing reads it yet: the main area arrives in STEP 6. Focus Mode, the one
 *                      feature that used to change this grid, was removed in v3.0 (spec section 10,
 *                      the subtracted list), so the prototype's `.workspace.focus-mode` rule is
 *                      deliberately not ported.
 *   mobileSidebarOpen  the sidebar drawer at <=900px, and the backdrop that dims the workspace
 *                      behind it.
 *
 * Defaults are the >1200px reading of spec section 7.2: all three columns visible. The <=1200px and
 * <=900px readings are not defaults but viewport facts, which is why `syncViewport()` exists:
 * `useResponsiveShell()` (src/layout/useShellLayout.ts) feeds it `matchMedia`, and the `@media`
 * blocks of Shell.css key off the same two widths.
 */

/** Sidebar column: `visible` = the 280px column, `collapsed` = `.no-sidebar`. */
export type SidebarVisibility = 'visible' | 'collapsed';

/** Right panel: `visible` = `.show-right`, `hidden` = `.no-right`. */
export type RightPanelVisibility = 'visible' | 'hidden';

/** Everything the shell grid depends on (spec sections 7.0 and 7.2). */
export interface LayoutState {
  sidebar: SidebarVisibility;
  right: RightPanelVisibility;
  split: boolean;
  mobileSidebarOpen: boolean;
  /**
   * Simple / Pro / Auto (spec section 7.1, row 5). Pro is the default. It is not a geometry - it
   * is the app's depth setting, and the panels that change behaviour with it read it from here:
   * the right panel hides Duel in Simple (spec section 7.10), and later steps hide the model name
   * in Simple (spec section 13.9). The mode switch persists no further than this store.
   */
  mode: AppMode;
  /**
   * The active theme. `<html data-theme>` is what actually restyles the app (src/lib/theme.ts is
   * the only writer) - this field is the mirror of it, so the moon/sun button, the Appearance tab
   * and the status bar can all read one source instead of the DOM.
   */
  theme: Theme;
}

/** The three depths of spec section 2.3, as the topbar's mode switch names them. */
export type AppMode = 'simple' | 'pro' | 'auto';

/**
 * The viewport facts the two responsive rules of spec section 7.2 are keyed off. Both come from
 * `matchMedia` in src/layout/useShellLayout.ts, and they mirror the two `@media` blocks of
 * src/layout/Shell.css exactly; changing one width means changing the other.
 */
export interface ShellViewport {
  /** `(max-width: 1200px)` - the right panel is off until something forces it back on. */
  narrowRightPanel: boolean;
  /** `(max-width: 900px)` - sidebar and right panel are drawers. */
  mobileDrawers: boolean;
}

/** The four actions of spec section 7.2, plus the viewport sync described above. */
export interface LayoutActions {
  /** Ctrl+B / `#toggleSidebar`: collapse or expand the sidebar column, and the drawer with it. */
  toggleSidebar: () => void;
  /** Ctrl+J / `#toggleRight`: `show-right` <-> `no-right` (spec section 7.1, row 10). */
  toggleRight: () => void;
  /** Force the right panel on - what a right-panel tab does at <=1200px (spec section 7.2). */
  showRight: () => void;
  /** Force it off. Idempotent, so the viewport sync can call it freely. */
  hideRight: () => void;
  /** Ctrl+\ / `#splitBtn`: Split view (spec section 9.15). State only in STEP 3. */
  toggleSplit: () => void;
  /** Open the <=900px sidebar drawer. */
  openMobileSidebar: () => void;
  /** Close it - the backdrop click of STEP 13 calls this. Idempotent. */
  closeMobileSidebar: () => void;
  /** Re-derive `right` and `mobileSidebarOpen` from the window width. Idempotent. */
  syncViewport: (viewport: ShellViewport) => void;
  /** Simple / Pro / Auto (spec section 7.1, row 5). */
  setMode: (mode: AppMode) => void;
  /** Dark <-> Light. `src/lib/theme.ts` writes `<html data-theme>`; this only records it. */
  setTheme: (theme: Theme) => void;
}

/** The store's full shape: state plus actions. */
export type LayoutStore = LayoutState & LayoutActions;

/**
 * The >1200px state of spec section 7.2: three columns, no split, no drawer. Exported so a test or
 * a later "reset layout" command can name the same values the store starts from.
 */
export const initialLayoutState: LayoutState = {
  sidebar: 'visible',
  right: 'visible',
  split: false,
  mobileSidebarOpen: false,
  /* Pro is the default mode (spec section 7.1, row 5) and dark the default theme (section 8.1). */
  mode: 'pro',
  theme: DEFAULT_THEME,
};

/**
 * The shell layout store.
 *
 * `toggleSidebar()` flips the column and the mobile drawer together, because the prototype's
 * `#toggleSidebar` handler toggles `no-sidebar` and `mobile-sidebar-open` in the same click
 * (design/ui-prototype.html, LAYOUT). Above 900px only the first class has a rule and below it only
 * the second one is visible, so one action is right at both widths and the two fields stay in step.
 */
export const useLayoutStore = create<LayoutStore>()((set, get) => ({
  ...initialLayoutState,

  toggleSidebar: () =>
    set((state) => ({
      sidebar: state.sidebar === 'visible' ? 'collapsed' : 'visible',
      mobileSidebarOpen: !state.mobileSidebarOpen,
    })),

  toggleRight: () => set((state) => ({ right: state.right === 'visible' ? 'hidden' : 'visible' })),

  /* The idempotent actions skip their write when the value is already right: `set` always creates a
     new state object, and a new object re-renders every subscriber. */
  showRight: () => {
    if (get().right !== 'visible') {
      set({ right: 'visible' });
    }
  },

  hideRight: () => {
    if (get().right !== 'hidden') {
      set({ right: 'hidden' });
    }
  },

  toggleSplit: () => set((state) => ({ split: !state.split })),

  setMode: (mode) => {
    if (get().mode !== mode) {
      set({ mode });
    }
  },

  setTheme: (theme) => {
    if (get().theme !== theme) {
      set({ theme });
    }
  },

  openMobileSidebar: () => {
    if (!get().mobileSidebarOpen) {
      set({ mobileSidebarOpen: true });
    }
  },

  closeMobileSidebar: () => {
    if (get().mobileSidebarOpen) {
      set({ mobileSidebarOpen: false });
    }
  },

  /**
   * Spec section 7.2, in code:
   *   >1200px  the right panel is part of the layout, so `right` is 'visible';
   *   <=1200px it is off until something forces it back (`show-right`);
   *   <=900px  both side regions are drawers, so the drawer starts closed too.
   * `sidebar` is a user preference and is left alone - at <=900px it has no rule at all.
   */
  syncViewport: ({ narrowRightPanel, mobileDrawers }) => {
    const state = get();
    const right: RightPanelVisibility = narrowRightPanel || mobileDrawers ? 'hidden' : 'visible';
    const mobileSidebarOpen = mobileDrawers ? false : state.mobileSidebarOpen;

    if (state.right === right && state.mobileSidebarOpen === mobileSidebarOpen) {
      return;
    }

    set({ right, mobileSidebarOpen });
  },
}));

/**
 * The class list of `#workspace` - the one place where store state becomes the shell's CSS contract
 * (spec sections 7.0 and 7.2). It mirrors `#workspace`'s class list in the prototype, except that
 * the prototype's two toggle handlers can leave it in a combination this never produces
 * (`show-right` and `no-right` together).
 *
 *   sidebar 'collapsed'  -> .no-sidebar            grid 280px -> 0
 *   right   'hidden'     -> .no-right              grid 400px -> 0
 *   right   'visible'    -> .show-right            forces the panel back at <=1200px
 *   mobileSidebarOpen    -> .mobile-sidebar-open   drawer in  (<=900px only)
 *   a drawer open        -> .mobile-backdrop       dim layer  (<=900px only)
 *
 * `mobile-backdrop` is emitted whenever a drawer is open, at any width: its rule lives inside the
 * <=900px media query in Shell.css, so above 900px the class is inert.
 *
 * `split` deliberately does not appear here - see the note on `LayoutState.split`.
 */
export function workspaceClassName(state: LayoutState): string {
  const classNames = ['workspace'];

  if (state.sidebar === 'collapsed') {
    classNames.push('no-sidebar');
  }

  if (state.right === 'hidden') {
    classNames.push('no-right');
  } else {
    classNames.push('show-right');
  }

  if (state.mobileSidebarOpen) {
    classNames.push('mobile-sidebar-open');
  }

  if (state.mobileSidebarOpen || state.right === 'visible') {
    classNames.push('mobile-backdrop');
  }

  return classNames.join(' ');
}

