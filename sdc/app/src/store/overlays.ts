import { create } from 'zustand';

/**
 * Overlay store - "which surface is open", and nothing else (spec sections 7.14 and 9.2-9.13).
 *
 * Six surfaces are reachable from the chrome: the command palette (9.2), the search overlay (9.4),
 * the Provider Hub (9.10), Settings (9.11), Add host (9.12) and the Permission dialog (9.13), plus
 * the keyboard reference of 9.1 (F1). Each is a flag (and, where the surface has tabs, the tab it
 * should open on).
 *
 * This is deliberately *not* event-sourced, and the reason is spec section 3.3's own rule: "persist
 * to localStorage ONLY for UI prefs". Which modal is up is presentation, not a fact about a host -
 * the daemon has no opinion about whether the hub is open, and putting it in the log would mean
 * every modal open is replayed on the next launch. The *content* those surfaces show is folded from
 * events; this file only remembers that they are on screen.
 *
 * The two popovers - the host switcher (7.1, row 3) and New chat (9.5) - need to know where they
 * hang from, so they carry an anchor: the trigger's `getBoundingClientRect()`, captured at click
 * time. They are rendered from src/App.tsx at that point so they can never be clipped by the region
 * the button lives in.
 */

/** Settings' seven tabs, in the order the modal lists them (spec section 9.11). */
export type SettingsTab =
  | 'general'
  | 'appearance'
  | 'keymap'
  | 'safety'
  | 'notifications'
  | 'backup'
  | 'about';

/** The Provider Hub's seven nav items, in order (spec section 9.10). */
export type HubTab =
  | 'all'
  | 'subscriptions'
  | 'api-keys'
  | 'local'
  | 'custom'
  | 'registry'
  | 'doctor';

/** Where a popover should appear: viewport coordinates, from the trigger's rect. */
export interface Anchor {
  x: number;
  y: number;
}

export interface OverlayState {
  settingsOpen: boolean;
  settingsTab: SettingsTab;
  hubOpen: boolean;
  hubTab: HubTab;
  paletteOpen: boolean;
  searchOpen: boolean;
  addHostOpen: boolean;
  /**
   * The host the Add-host dialog is **already about**, when it was opened to answer a question about a
   * host rather than to add one (0.7.13): a key that needs trusting, or a re-pin after a change.
   *
   * One surface for both entry points on purpose - the dialog already knows how to
   * show a fingerprint, a sentence and a button, and a second "Manage host" modal would be a second
   * implementation of the trust card that could drift from the first.
   */
  addHostHostId: string | null;
  permissionOpen: boolean;
  /** The F1 keyboard reference (spec section 9.1). */
  keymapOpen: boolean;
  /**
   * The Connect modal (spec section 9.10): `login` drives a CLI's own sign-in, `api` takes a key and
   * picks the model to use it with. Which provider it is about is part of the state, because the
   * modal is opened *from* a provider card rather than from the nav.
   */
  connectOpen: boolean;
  connectProviderId: string | null;
  connectMode: 'login' | 'api';
  /** Non-null while the host switcher is up, carrying the point it hangs from. */
  hostSwitcher: Anchor | null;
  /** Non-null while the New chat popover is up. */
  newChat: Anchor | null;
  /**
   * The host whose folders are being browsed (0.7.13), or `null`.
   *
   * A remote folder cannot be chosen with the native picker - that dialog shows *this* machine's
   * filesystem, and a VPS is not on it - so `fs.list` on that host is the browser, and this flag says
   * which host it is about.
   */
  remoteFolderHostId: string | null;
  /**
   * The chat the chosen remote folder should re-point (0.10.0), or `null` when the browser was opened
   * to land in a new or empty chat. `Change folder` on a VPS chat sets it: the chip's promise is
   * "this chat, another folder", so the choice must bind *this* chat rather than open another one.
   */
  remoteFolderSessionId: string | null;
  /** Start-from-scratch dialog (0.9.0). */
  newProjectOpen: boolean;
}

export interface OverlayActions {
  /** `#openSettings` and the brand mark: the latter asks for the About tab (spec section 7.1, row 1). */
  openSettings: (tab?: SettingsTab) => void;
  closeSettings: () => void;
  openHub: (tab?: HubTab) => void;
  closeHub: () => void;
  openPalette: () => void;
  closePalette: () => void;
  openSearch: () => void;
  closeSearch: () => void;
  openAddHost: (hostId?: string) => void;
  closeAddHost: () => void;
  openPermission: () => void;
  closePermission: () => void;
  openKeymap: () => void;
  closeKeymap: () => void;
  /** A provider card's Connect button: `login` for a subscription, `api` for a key and a model. */
  openConnect: (providerId: string, mode: 'login' | 'api') => void;
  closeConnect: () => void;
  openHostSwitcher: (anchor: Anchor) => void;
  closeHostSwitcher: () => void;
  openNewChat: (anchor: Anchor) => void;
  closeNewChat: () => void;
  /** The remote folder browser, for a host that has no native picker (0.7.13). */
  openRemoteFolder: (hostId: string, sessionId?: string) => void;
  closeRemoteFolder: () => void;
  /** Start from scratch (0.9.0): one dialog that makes the folder, the project and the first turn. */
  openNewProject: () => void;
  closeNewProject: () => void;
  /** Esc: every overlay closes at once (spec section 8.6). */
  closeAll: () => void;
}

const initialOverlayState: OverlayState = {
  newProjectOpen: false,
  settingsOpen: false,
  settingsTab: 'general',
  hubOpen: false,
  hubTab: 'all',
  paletteOpen: false,
  searchOpen: false,
  addHostOpen: false,
  addHostHostId: null,
  permissionOpen: false,
  keymapOpen: false,
  connectOpen: false,
  connectProviderId: null,
  connectMode: 'api',
  hostSwitcher: null,
  newChat: null,
  remoteFolderHostId: null,
  remoteFolderSessionId: null,
};

export const useOverlayStore = create<OverlayState & OverlayActions>()((set) => ({
  ...initialOverlayState,

  openSettings: (tab = 'general') => set({ settingsOpen: true, settingsTab: tab }),

  closeSettings: () => set({ settingsOpen: false }),

  openHub: (tab = 'all') => set({ hubOpen: true, hubTab: tab }),

  closeHub: () => set({ hubOpen: false }),

  openPalette: () => set({ paletteOpen: true }),

  closePalette: () => set({ paletteOpen: false }),

  openSearch: () => set({ searchOpen: true }),

  closeSearch: () => set({ searchOpen: false }),

  openAddHost: (hostId) => set({ addHostOpen: true, addHostHostId: hostId ?? null }),

  closeAddHost: () => set({ addHostOpen: false, addHostHostId: null }),

  openPermission: () => set({ permissionOpen: true }),

  closePermission: () => set({ permissionOpen: false }),

  openKeymap: () => set({ keymapOpen: true }),

  closeKeymap: () => set({ keymapOpen: false }),

  openConnect: (providerId, mode) => set({ connectOpen: true, connectProviderId: providerId, connectMode: mode }),

  closeConnect: () => set({ connectOpen: false, connectProviderId: null }),

  openHostSwitcher: (anchor) => set({ hostSwitcher: anchor, newChat: null }),

  closeHostSwitcher: () => set({ hostSwitcher: null }),

  openNewChat: (anchor) => set({ newChat: anchor, hostSwitcher: null }),

  closeNewChat: () => set({ newChat: null }),

  openRemoteFolder: (hostId, sessionId) =>
    set({ remoteFolderHostId: hostId, remoteFolderSessionId: sessionId ?? null }),
  openNewProject: () => set({ newProjectOpen: true }),
  closeNewProject: () => set({ newProjectOpen: false }),

  closeRemoteFolder: () => set({ remoteFolderHostId: null, remoteFolderSessionId: null }),

  closeAll: () => set({ ...initialOverlayState }),
}));

/**
 * Reads an element's viewport rect into the anchor a popover wants, so every trigger - the topbar
 * host pill, a `.host-add` button, the status bar's host item - positions its popover the same way
 * (the prototype aligns the popover with the trigger's left edge and hangs it 6px below).
 */
export function anchorBelow(element: Element): Anchor {
  const rect = element.getBoundingClientRect();

  return { x: rect.left, y: rect.bottom + 6 };
}
