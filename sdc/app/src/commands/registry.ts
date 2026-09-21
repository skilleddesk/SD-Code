import { ensureSplitSecondary, usePrefsStore } from '../store/prefs';
import { sessionActions } from '../store/sessions';
import { useAppStore } from '../store/store';
import { useLayoutStore } from '../store/layout';
import { nextEngine, nextTier, useModelStore } from '../store/model';
import { useOverlayStore } from '../store/overlays';
import {
  changeFolder,
  closeFolder,
  forceKillTurn,
  interruptTurn,
  openFolder,
  refreshDirectory,
  resolvePermission,
  redoRewind,
  rewindTo,
  startTurn,
} from '../store/intents';
import { nameOf } from '../lib/picker';
import { toast } from '../store/toast';
import { strings } from '../strings';

/**
 * The command registry - spec section 9.1 and principle P7 ("one registry, no divergence").
 *
 * Every shortcut and every palette row is one entry here: `{ id, label, hint, icon, keys, group,
 * run, when }`. Nothing else in the app defines a key binding, which is the only way the three
 * surfaces that *show* shortcuts - the palette, the F1 reference and Settings → Keymap - can be
 * guaranteed to agree: they all render `COMMANDS`.
 *
 * `keys` holds normalized specs (`ctrl+k`, `shift+enter`, `alt+m`). Normalizing once, here, is what
 * lets the keyboard listener be a single `keydown` handler with no per-command branching.
 *
 * `inInput` answers spec section 9.1's focus rule - "typing inside textarea/input does not trigger
 * Ctrl+letter unless documented". The documented exceptions carry `inInput: true`.
 */

export type CommandGroup = 'global' | 'session' | 'model' | 'approval' | 'timeline' | 'actions';

export type CommandIcon =
  | 'plus' | 'plug' | 'serverPlus' | 'columns' | 'search' | 'check' | 'stethoscope' | 'settings'
  | 'keyboard' | 'panelLeft' | 'panelRight' | 'x' | 'zap' | 'brain' | 'clock' | 'ban' | 'shield'
  | 'eye' | 'folder' | 'fork';

export interface Command {
  /** Stable id: what a keymap override file would name. */
  id: string;
  label: string;
  /** Key label shown at the right of a palette row and in the reference: `Ctrl K`. */
  hint?: string;
  icon?: CommandIcon;
  group: CommandGroup;
  /** Normalized specs. Empty means palette-only. */
  keys?: readonly string[];
  /** Fires while a text field has focus (the documented exceptions). */
  inInput?: boolean;
  /** Shown in the palette; true unless stated otherwise. */
  palette?: boolean;
  when?: () => boolean;
  run: () => void;
}

/** Which section of the F1 reference and the Keymap tab a command belongs to. */
export const GROUP_LABEL: Record<CommandGroup, string> = {
  global: strings.keymap.groups.global,
  session: strings.keymap.groups.session,
  model: strings.keymap.groups.model,
  approval: strings.keymap.groups.approval,
  timeline: strings.keymap.groups.timeline,
  actions: strings.keymap.groups.actions,
};

/** The groups in the order the F1 reference lists them. */
export const GROUP_ORDER: readonly CommandGroup[] = [
  'global',
  'session',
  'model',
  'approval',
  'timeline',
  'actions',
];

/** The helper the session commands act on: the newest turn in the log, if any. */
function latestTurn(): { turnId: string; sessionId: string } | null {
  const state = useAppStore.getState();
  const turn = state.turns.at(-1);

  return turn === undefined ? null : { turnId: turn.id, sessionId: turn.sessionId };
}

function mainSession(): string {
  return usePrefsStore.getState().activeTab ?? 's1';
}

/** Resolves the open approval dialog with one of spec section 9.13's four decisions. */
function decide(decision: 'allow_once' | 'always_allow' | 'deny' | 'show_me'): void {
  const permission = useAppStore.getState().permission;

  if (permission) {
    void resolvePermission(permission.id, decision);
  }

  useOverlayStore.getState().closePermission();
}

/** `J`/`K`, `G`, `Shift+G`: move the timeline's selection (spec section 9.1). */
function moveTimeline(step: number | 'start' | 'end'): void {
  const turns = useAppStore.getState().turns;

  if (turns.length === 0) {
    return;
  }

  const index =
    step === 'start' ? 0 : step === 'end' ? turns.length - 1 : Math.max(0, turns.length - 1 + step);
  const turn = turns[index];

  if (turn) {
    useAppStore.setState({ activeTurnId: turn.id });
    toast(`Turn ${turn.turnNumber} · ${turn.engine}`);
  }
}

/**
 * The active chat's folder, if it has one - what `folder.change` and `folder.close` act on (0.7.6).
 *
 * It reads the two stores the prompt area's chip also reads, so a command in the palette and the chip on
 * screen always agree about which folder they mean. `null` covers both reasons there may be nothing to do:
 * no chat is open, or the chat has no folder yet.
 */
function activeFolder(): { sessionId: string; projectId: string; name: string; root: string } | null {
  const activeTab = usePrefsStore.getState().activeTab;

  if (activeTab === null) {
    return null;
  }

  for (const host of useAppStore.getState().hosts) {
    const session = host.sessions.find((candidate) => candidate.id === activeTab);

    if (session?.projectId) {
      return {
        sessionId: session.id,
        projectId: session.projectId,
        name: nameOf(session.projectRoot ?? session.projectId),
        /* The tree's refresh needs the path itself, not the name the chip shows. */
        root: session.projectRoot ?? '',
      };
    }
  }

  return null;
}

/**
 * A chat's title by its id - for a palette row that says what it is about to fork.
 *
 * The row's *label* is the same for every chat (`Fork this chat`), so the title is only used for the
 * sentence the fork raises; `null` means the id is not in the tree at all, in which case there is nothing
 * to fork and the command does nothing.
 */
function activeChat(sessionId: string): string | null {
  for (const host of useAppStore.getState().hosts) {
    const session = host.sessions.find((candidate) => candidate.id === sessionId);

    if (session !== undefined) {
      return session.title;
    }
  }

  return null;
}

/** One row per command. The palette, the F1 reference and the Keymap tab all render this array. */
export const COMMANDS: readonly Command[] = [
  /* ---------------------------------------------------------------- Global (10) */
  { id: 'palette.open', label: strings.topbar.palette.title, hint: 'Ctrl K', icon: 'search', group: 'global', keys: ['ctrl+k', 'meta+k'], inInput: true, run: () => useOverlayStore.getState().openPalette() },
  { id: 'search.open', label: 'Search everything', hint: 'Ctrl P', icon: 'search', group: 'global', keys: ['ctrl+p', 'meta+p'], inInput: true, run: () => useOverlayStore.getState().openSearch() },
  { id: 'chat.new', label: strings.sidebar.newChat, hint: 'Ctrl N', icon: 'plus', group: 'global', keys: ['ctrl+n', 'meta+n'], run: () => useOverlayStore.getState().openNewChat({ x: 12, y: 52 }) },
  { id: 'tab.close', label: strings.tabs.close, hint: 'Ctrl W', icon: 'x', group: 'global', keys: ['ctrl+w', 'meta+w'], run: () => { const prefs = usePrefsStore.getState(); if (prefs.activeTab !== null) { prefs.closeTab(prefs.activeTab); } } },
  { id: 'chat.fork', label: strings.sidebar.actions.fork, icon: 'fork', group: 'actions', when: () => usePrefsStore.getState().activeTab !== null, run: () => { const id = usePrefsStore.getState().activeTab; if (id === null) { return; } const found = activeChat(id); if (found !== null) { sessionActions.forkSession(id, found); } } },
  { id: 'sidebar.toggle', label: strings.topbar.sidebar.title, hint: 'Ctrl B', icon: 'panelLeft', group: 'global', keys: ['ctrl+b', 'meta+b'], run: () => useLayoutStore.getState().toggleSidebar() },
  { id: 'panel.toggle', label: strings.topbar.right.title, hint: 'Ctrl J', icon: 'panelRight', group: 'global', keys: ['ctrl+j', 'meta+j'], run: () => useLayoutStore.getState().toggleRight() },
  { id: 'split.toggle', label: strings.tabs.split.title, hint: 'Ctrl \\', icon: 'columns', group: 'global', keys: ['ctrl+\\', 'meta+\\'], inInput: true, run: () => { useLayoutStore.getState().toggleSplit(); ensureSplitSecondary(); toast(useLayoutStore.getState().split ? strings.tabs.split.on : strings.tabs.split.off); } },
  { id: 'settings.open', label: strings.topbar.settings.title, hint: 'Ctrl ,', icon: 'settings', group: 'global', keys: ['ctrl+,', 'meta+,'], inInput: true, run: () => useOverlayStore.getState().openSettings() },
  { id: 'keymap.open', label: strings.keymap.title, hint: 'F1', icon: 'keyboard', group: 'global', keys: ['f1'], inInput: true, run: () => useOverlayStore.getState().openKeymap() },
  { id: 'overlay.close', label: 'Close overlay', hint: 'Esc', icon: 'x', group: 'global', keys: ['escape'], inInput: true, palette: false, when: () => useAppStore.getState().permission === null, run: () => useOverlayStore.getState().closeAll() },

  /* ---------------------------------------------------------------- Session (6) */
  { id: 'turn.interrupt', label: 'Interrupt', hint: 'Esc', icon: 'ban', group: 'session', keys: ['escape'], inInput: true, run: () => { const turn = latestTurn(); if (turn) { void interruptTurn(turn.turnId); } else { toast(strings.prompt.interrupt); } } },
  { id: 'turn.kill', label: 'Force kill', hint: 'Ctrl Shift Esc', icon: 'ban', group: 'session', keys: ['ctrl+shift+escape', 'meta+shift+escape'], inInput: true, run: () => { const turn = latestTurn(); if (turn) { void forceKillTurn(turn.turnId); } } },
  { id: 'turn.rewind', label: 'Rewind last turn', hint: 'Ctrl Z', icon: 'clock', group: 'session', keys: ['ctrl+z', 'meta+z'], run: () => { const checkpoint = useAppStore.getState().checkpoints[0]; if (checkpoint) { void rewindTo(mainSession(), `turn-${checkpoint.turn}`); } } },
  { id: 'turn.redo', label: 'Redo', hint: 'Ctrl Shift Z', icon: 'clock', group: 'session', keys: ['ctrl+shift+z', 'meta+shift+z'], run: () => void redoRewind(mainSession()) },
  { id: 'timemachine.open', label: 'Time Machine', hint: 'Ctrl E', icon: 'clock', group: 'session', keys: ['ctrl+e', 'meta+e'], run: () => useLayoutStore.getState().showRight() },
  { id: 'verify.run', label: 'Run verify', hint: 'Ctrl Enter', icon: 'check', group: 'session', keys: ['ctrl+enter', 'meta+enter'], inInput: true, run: () => toast(strings.rightPanel.verify.result) },

  /* ---------------------------------------------------------------- Model (2) */
  { id: 'tier.cycle', label: 'Cycle tier', hint: 'Alt M', icon: 'brain', group: 'model', keys: ['alt+m'], run: () => useModelStore.getState().setTier(nextTier(useModelStore.getState().tier)) },
  { id: 'engine.cycle', label: 'Cycle engine', hint: 'Alt E', icon: 'zap', group: 'model', keys: ['alt+e'], run: () => useModelStore.getState().setEngine(nextEngine(useModelStore.getState().engine)) },

  /* ---------------------------------------------------------------- Approval (6) */
  { id: 'permission.default', label: 'Default action', hint: 'Enter', icon: 'check', group: 'approval', keys: ['enter'], inInput: true, palette: false, when: () => useAppStore.getState().permission !== null, run: () => decide(useAppStore.getState().permission?.risk === 'DANGEROUS' ? 'deny' : 'allow_once') },
  { id: 'permission.allow', label: 'Allow once', hint: 'A', icon: 'check', group: 'approval', keys: ['a'], inInput: true, palette: false, when: () => useAppStore.getState().permission !== null, run: () => decide('allow_once') },
  { id: 'permission.always', label: 'Always allow', hint: 'Shift A', icon: 'shield', group: 'approval', keys: ['shift+a'], inInput: true, palette: false, when: () => useAppStore.getState().permission?.risk === 'MUTATING', run: () => decide('always_allow') },
  { id: 'permission.show', label: 'Show me the file', hint: 'S', icon: 'eye', group: 'approval', keys: ['s'], inInput: true, palette: false, when: () => useAppStore.getState().permission !== null, run: () => decide('show_me') },
  { id: 'permission.deny', label: 'Deny', hint: 'D / Esc', icon: 'ban', group: 'approval', keys: ['d', 'escape'], inInput: true, palette: false, when: () => useAppStore.getState().permission !== null, run: () => decide('deny') },

  /* ---------------------------------------------------------------- Timeline (6) */
  { id: 'timeline.next', label: 'Next turn', hint: 'J', group: 'timeline', keys: ['j'], palette: false, run: () => moveTimeline(1) },
  { id: 'timeline.previous', label: 'Previous turn', hint: 'K', group: 'timeline', keys: ['k'], palette: false, run: () => moveTimeline(-1) },
  { id: 'timeline.toggle', label: 'Expand / collapse turn', hint: 'O', group: 'timeline', keys: ['o'], palette: false, run: () => toast(strings.turns.error.explainMoreToast) },
  { id: 'timeline.start', label: 'First turn', hint: 'G', group: 'timeline', keys: ['g'], palette: false, run: () => moveTimeline('start') },
  { id: 'timeline.end', label: 'Last turn', hint: 'Shift G', group: 'timeline', keys: ['shift+g'], palette: false, run: () => moveTimeline('end') },
  { id: 'timeline.copy', label: 'Copy summary', hint: 'Y', group: 'timeline', keys: ['y'], palette: false, run: () => { const turn = useAppStore.getState().turns.at(-1); const summary = turn?.summary ?? ''; if (summary === '') { toast(strings.timeline.nothingToCopy); return; } void navigator.clipboard?.writeText(summary); toast(summary); } },

  /* ---------------------------------------------------------------- Actions (palette only) */
  { id: 'host.add', label: strings.sidebar.addHost, icon: 'serverPlus', group: 'actions', run: () => useOverlayStore.getState().openAddHost() },
  { id: 'providers.open', label: 'Connect a provider / model', icon: 'plug', group: 'actions', run: () => useOverlayStore.getState().openHub() },
  { id: 'doctor.open', label: 'Run environment doctor', icon: 'stethoscope', group: 'actions', run: () => useOverlayStore.getState().openHub('doctor') },
  { id: 'folder.open', label: strings.main.noProject.action, icon: 'folder', group: 'actions', run: () => void openFolder() },
  { id: 'folder.change', label: strings.folder.change, icon: 'folder', group: 'actions', when: () => activeFolder() !== null, run: () => { const current = activeFolder(); if (current !== null) { void changeFolder(current.sessionId); } } },
  { id: 'folder.close', label: 'Close this folder', icon: 'folder', group: 'actions', when: () => activeFolder() !== null, run: () => { const current = activeFolder(); if (current !== null) { void closeFolder(current.projectId, current.name); } } },
  { id: 'files.refresh', label: strings.files.refresh, icon: 'folder', group: 'actions', when: () => activeFolder() !== null, run: () => { const current = activeFolder(); if (current !== null) { void refreshDirectory(current.root); } } },
];

/* ------------------------------------------------------------------------------------------------
 * Key specs and lookup
 * ---------------------------------------------------------------------------------------------- */

/** The key names `KeyboardEvent.key` uses that need a friendlier spec. */
const KEY_ALIASES: Record<string, string> = {
  esc: 'escape',
  ' ': 'space',
  arrowup: 'up',
  arrowdown: 'down',
  arrowleft: 'left',
  arrowright: 'right',
  '\\': '\\',
};

/** Normalizes an event into the spec form the registry is written in: `ctrl+shift+escape`. */
export function keySpec(event: KeyboardEvent): string {
  const raw = event.key.toLowerCase();
  const key = KEY_ALIASES[raw] ?? raw;
  const parts: string[] = [];

  if (event.ctrlKey) parts.push('ctrl');
  if (event.altKey) parts.push('alt');
  if (event.shiftKey) parts.push('shift');
  if (event.metaKey) parts.push('meta');

  parts.push(key);

  return parts.join('+');
}

/** The command a spec belongs to, or undefined. Used by the reference and by `useKeys`. */
export function commandForSpec(spec: string): Command | undefined {
  return COMMANDS.find((command) => (command.keys ?? []).includes(spec));
}

/** Every command that fires on a spec and whose `when()` currently passes. */
export function commandsForSpec(spec: string): Command[] {
  return COMMANDS.filter(
    (command) => (command.keys ?? []).includes(spec) && (command.when === undefined || command.when()),
  );
}

/** The reference/Keymap tab, grouped and in order. Commands without keys are omitted. */
export function commandsByGroup(): { group: CommandGroup; label: string; rows: Command[] }[] {
  return GROUP_ORDER.map((group) => ({
    group,
    label: GROUP_LABEL[group],
    rows: COMMANDS.filter((command) => command.group === group && (command.keys ?? []).length > 0),
  })).filter((section) => section.rows.length > 0);
}

/** The palette's list: everything the palette is allowed to show. */
export function paletteCommands(): Command[] {
  return COMMANDS.filter((command) => command.palette !== false);
}

/**
 * Fuzzy match, in the order the answers are useful: an exact prefix first, then a word-boundary
 * prefix (`cp` → `Connect a provider`), then a subsequence (`set` → `Open settings`). Returns the
 * score so the palette can sort, and `null` for no match at all.
 */
export function fuzzyScore(query: string, label: string): number | null {
  const haystack = label.toLowerCase();
  const needle = query.trim().toLowerCase();

  if (needle === '') {
    return 0;
  }

  if (haystack.startsWith(needle)) {
    return 1000 - haystack.length;
  }

  const boundary = haystack
    .split(/[\s/]+/)
    .map((word) => word.slice(0, 1))
    .join('');

  if (boundary.startsWith(needle)) {
    return 800 - haystack.length;
  }

  if (haystack.includes(needle)) {
    return 600 - haystack.indexOf(needle);
  }

  /* Subsequence: every character in order, gaps allowed. */
  let cursor = 0;

  for (const character of needle) {
    const found = haystack.indexOf(character, cursor);

    if (found < 0) {
      return null;
    }

    cursor = found + 1;
  }

  return 200 - haystack.length;
}

/** The palette's filtered list, best match first. Stable for equal scores (input order wins). */
export function matchCommands(query: string): Command[] {
  return paletteCommands()
    .map((command, index) => ({ command, index, score: fuzzyScore(query, command.label) }))
    .filter((entry): entry is { command: Command; index: number; score: number } => entry.score !== null)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((entry) => entry.command);
}

export { startTurn };

