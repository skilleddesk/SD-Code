import type {
  CheckpointRecord,
  ConsoleErrorEvent,
  PermissionDecision,
  PermissionRisk,
  ProviderKind,
  ProviderLifecycle,
  RegistryModel,
  SdcpEvent,
  TierName,
} from '../../../protocol/types';

/**
 * The reducer's state shape and the UI's view types (master spec sections 3.3, 5.4).
 *
 * Spec section 3.3 is the rule this file exists for: **the event log is the single source of
 * truth**. Nothing in the app holds a fact that cannot be re-derived by folding the log, and no
 * component writes these fields - it dispatches an intent, the daemon appends an event, and the
 * reducer projects a new state.
 *
 * `AppState` is therefore deliberately *derived*: `hosts` is the host/session tree the sidebar
 * renders, `providers` is the Provider Hub's list, `turns` is the turn stream, and every one of them
 * is produced by folding `SdcpEvent`s. Two fields are bookkeeping rather than view - `seq`, the
 * highest sequence number folded in (a gap is a replay request, not a lost paint), and `lastTs`, the
 * daemon clock of that event. The reducer is pure, so it may never call `Date.now()`; the clock is
 * *passed in* as part of the event.
 *
 * The protocol types come from `protocol/types.ts`. That directory is not a workspace package yet -
 * the schema generator of spec section 5.10 will make it one - so the import is relative on purpose.
 */

/* ------------------------------------------------------------------------------------------------
 * The tree the sidebar, tab strip and status bar read
 * ---------------------------------------------------------------------------------------------- */

/** A session as the sidebar draws it. `attention` is what floats a row to the top of its host. */
export interface SessionView {
  id: string;
  title: string;
  prompt: string;
  state: 'idle' | 'running' | 'waiting' | 'success' | 'error';
  minutesAgo: number;
  unread: number;
  attention?: 'awaiting_approval' | 'stuck' | 'budget_stop';
  /**
   * The folder this chat works in (0.7.6), or `null` for a chat that has no project.
   *
   * `projectRoot` is the absolute path the engines are started in - which is why the prompt area shows it:
   * "which directory am I in" is the one question a coding agent's user asks first, and before 0.7.6 the
   * answer was "whatever folder the daemon was started in", which the app could not even display.
   */
  projectId?: string | null;
  projectRoot?: string | null;
}

/**
 * A folder a chat can work in (0.7.6) - one row of `project.list`.
 *
 * `chats` is how many chats are bound to it, which is what says whether closing the folder will unbind
 * anything (`project.remove` leaves the conversations alone).
 */
export interface ProjectView {
  id: string;
  hostId: string;
  root: string;
  name: string;
  chats: number;
}

export interface HostView {
  id: string;
  name: string;
  type: 'local' | 'vps';
  status: 'connected' | 'untrusted' | 'degraded' | 'offline' | 'connecting';
  /** sdcd version on the far side; empty while the host is still connecting. */
  sdcd: string;
  /** `macOS 15.1 · arm64` - the About tab's "This host" line. */
  platform: string;
  /**
   * What just happened to this host, in words (0.7.13).
   *
   * `platform` is the machine line; this is the sentence - `root@vps is reachable`, `copying SDC's key
   * with that password…`, or, for a host SDC has never seen before, the question that ends with the
   * fingerprint. It is shown on the host's own card rather than in a toast, because a toast is written
   * to the log and replayed on every launch.
   */
  detail: string;
  /** The fingerprint an `untrusted` host is waiting to be trusted with - what `host.trust` takes. */
  hostKey: string;
  /** The fingerprint already pinned for this host (0.7.13), from its row - empty when there is none. */
  pinned: string;
  /** The `user@host:port` it was added with, for the card and the tooltip (0.7.13). */
  address: string;
  sessions: SessionView[];
}

/* ------------------------------------------------------------------------------------------------
 * Providers (spec section 9.10)
 * ---------------------------------------------------------------------------------------------- */

export interface ProviderView {
  id: string;
  name: string;
  kind: ProviderKind;
  status: ProviderLifecycle;
  detail: string;
  account: string | null;
  /** Slug for the gradient logo: `claude`, `openai`, `gemini`, `deepseek`, `groq`, `ollama`, … */
  logo: string;
  /** One letter, drawn inside the logo circle. */
  initial: string;
  /** Custom-endpoint only: the URL and the protocol it speaks. */
  url?: string;
  protocol?: string;
}

/* ------------------------------------------------------------------------------------------------
 * The turn stream (spec section 7.5)
 * ---------------------------------------------------------------------------------------------- */

export interface ToolView {
  callId: string;
  tool: 'read' | 'edit' | 'run';
  name: string;
  target: string;
  status: 'running' | 'done' | 'failed';
  meta: string;
  diff: { lineNumber: string; text: string; change: 'add' | 'rem' }[];
  output: { level: 'ok' | 'fail' | 'dim'; text: string }[];
}

export interface ErrorView {
  title: string;
  explanation: string;
  source?: string;
  fixable: boolean;
}

export interface TurnView {
  id: string;
  /** The turn's ordinal in its session - what the Time Machine and a rewind address (14). */
  turnNumber: number;
  sessionId: string;
  engine: string;
  model: string;
  tier: TierName;
  /** What the user asked. From `TurnStarted`, so it survives a reload. */
  prompt: string;
  /** The engine's answer as it streams in; `aria-live="polite"` reads it. */
  text: string;
  thinking: string;
  /**
   * How long the engine has spent thinking, in milliseconds, over every stretch of it that has ended.
   * Measured between the log's own timestamps: the first `ThinkingDelta` of a stretch, and the first
   * event after it that is not one (an answer delta, a tool call, the end of the turn).
   */
  thinkingMs: number;
  /** The `ts` of the stretch of thinking still going on, or `null` when the engine is not thinking. */
  thinkingSince: string | null;
  /** The agent's checklist (`PlanUpdated`, v4), newest version; empty for a turn without one. */
  plan: PlanStepView[];
  status: 'running' | 'stuck' | 'done' | 'failed';
  /** Milliseconds without output, set by `StuckDetected` (spec section 12.9). */
  stuckForMs: number;
  tools: ToolView[];
  error?: ErrorView;
  summary: string;
  meta: string;
  pass: boolean | null;
}

/* ------------------------------------------------------------------------------------------------
 * The five differentiators' view state (spec sections 2.5, 14-16)
 * ---------------------------------------------------------------------------------------------- */

/** One step of an agent's plan card. */
export interface PlanStepView {
  text: string;
  status: 'pending' | 'in_progress' | 'done';
}

/** One row of the Time Machine tab: a checkpoint the rewind machinery can restore. */
export interface CheckpointView {
  id: string;
  sessionId: string;
  /** The turn that wrote it, from the event's envelope - `null` for a checkpoint a Save or a command made. */
  turnId: string | null;
  turn: number;
  /** Relative age, computed once by the producer: `now`, `2 min ago`. */
  when: string;
  title: string;
  thumbnail: string | null;
  filesHash: string;
}

/** Spec section 16.5: what a mid-turn engine switch has to carry across. */
export interface BridgeFrame {
  sessionId: string;
  turnId: string;
  from: string;
  to: string;
  model: string;
  reason: string;
}

/** Spec section 16.6: the two panes of a duel, plus which one won. */
export interface DuelView {
  id: string;
  sessionId: string;
  prompt: string;
  engines: string[];
  /** The two runs' results; empty until both engines have answered. */
  panes: DuelPaneView[];
  kept: string | null;
  resolved: boolean;
}

/** One engine's run inside a duel, as the Duel tab draws it. */
export interface DuelPaneView {
  engine: string;
  model: string;
  time: string;
  cost: string;
  pass: boolean;
  headline: string;
  files: string[];
}

/** Spec section 15.4: one preview console line, deduplicated by source. */
export interface ConsoleLine {
  level: 'error' | 'warn' | 'info';
  message: string;
  source: string;
  file: string;
  line: number;
  count: number;
}

/* ------------------------------------------------------------------------------------------------
 * Overlay state that is data, not geometry
 * ---------------------------------------------------------------------------------------------- */

export interface ToastRecord {
  id: number;
  message: string;
  action: string | null;
  /** Spec section 9.14: 3000ms, or 10000 for a rewind's `Undo this`. */
  holdMs: number;
}

/** Spec section 9.13: the approval dialog's content, plus the risk that decides its default button. */
export interface PermissionView {
  id: string;
  sessionId: string;
  turnId: string | null;
  title: string;
  sub: string;
  action: string;
  target: string;
  risk: PermissionRisk;
  explain: string;
  checkpointId: string | null;
}

export interface DoctorCheckView {
  id: string;
  label: string;
  state: 'ok' | 'warn' | 'fail';
  detail: string;
  fix: string | null;
}

/* ------------------------------------------------------------------------------------------------
 * The state itself
 * ---------------------------------------------------------------------------------------------- */

export interface AppState {
  /** Highest `notification.seq` folded in; 0 before the first event. */
  seq: number;
  /** The daemon clock of that event (RFC 3339). The reducer never reads a wall clock itself. */
  lastTs: string;

  /**
   * The host-grouped session tree (spec section 7.3). Tabs, the focused session and the shell
   * layout are NOT here - they are UI preferences and live in src/store/prefs.ts (spec section 3.3).
   */
  hosts: HostView[];

  /** The folders chats can work in (0.7.6), oldest first, as `project.list` reports them. */
  projects: ProjectView[];

  providers: ProviderView[];
  registry: RegistryModel[];
  toasts: ToastRecord[];
  toastsIssued: number;

  turns: TurnView[];
  /** The newest turn's id; the turn stream's "live" row. */
  activeTurnId: string | null;

  checkpoints: CheckpointView[];
  /** Newest first; `redo` pops from here (spec section 14). */
  rewindStack: CheckpointView[];
  /** Spec section 16.5: engine switches that carried a conversation across. */
  bridges: BridgeFrame[];

  duels: DuelView[];
  console: ConsoleLine[];

  permission: PermissionView | null;
  /** Decisions taken this session, keyed by permission id - what `always_allow` learns from. */
  resolvedPermissions: Record<string, PermissionDecision>;

  /** Spec section 9.10's environment doctor, newest run per host. */
  doctor: Record<string, DoctorCheckView[]>;
}

/* ------------------------------------------------------------------------------------------------
 * The notification envelope the reducer folds
 * ---------------------------------------------------------------------------------------------- */

/** `SdcpEvent` plus the envelope fields the reducer needs. */
export interface AppEvent {
  seq: number;
  ts: string;
  event: SdcpEvent;
  sessionId?: string | null;
  turnId?: string | null;
}

/** Re-exported so a caller has one place to import the protocol's types from. */
export type { CheckpointRecord, ConsoleErrorEvent, PermissionDecision, PermissionRisk };
export type { ProviderKind, ProviderLifecycle, RegistryModel, SdcpEvent };
