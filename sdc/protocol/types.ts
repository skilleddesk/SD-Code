/**
 * SDCP 0.1 — TypeScript mirror of `protocol/sdcp.schema.json` (spec section 5).
 *
 * HAND-WRITTEN MIRROR, ON PURPOSE. Spec section 5.10 wants the JSON Schema to be the single source
 * of truth and the Rust/TypeScript types to be *generated* from it. The generator needs the v2.0
 * document's tooling decision, which is not settled yet, so this file is written by hand and kept
 * structurally identical to the schema: same names, same optionality, same enums. When the
 * generator lands it replaces this file; the schema is the file to change in the meantime
 * (`protocol/README.md`, rule 1).
 *
 * The three wire shapes are `Envelope`, `Response` and `Notification`. Everything else in this
 * file is either an event payload (what the reducer consumes) or a method's params/result (what
 * `lib/sdcp.ts` types the calls with). Nothing here imports from `app/src`: the protocol has to be
 * usable by the Tauri bridge and by a future non-React consumer without dragging the UI in.
 */

/** Envelope/SDCP version. Moves only by the migration rules of spec section 5.7. */
export const SDCP_VERSION = '0.1';

export type ProtocolVersion = typeof SDCP_VERSION;

/** Machine-readable failure codes. The UI never parses `message` (principle P3). */
export type ErrorCode =
  | 'bad_request'
  | 'not_found'
  | 'not_ready'
  | 'permission_denied'
  | 'blocked_path'
  | 'engine_failed'
  | 'engine_stuck'
  | 'budget_exceeded'
  | 'cancelled'
  | 'unsupported'
  | 'internal';

export interface SdcpError {
  code: ErrorCode;
  message: string;
  data?: Record<string, unknown>;
  retryable?: boolean;
}

/** A request. `id` is echoed by the matching response, which is what correlates them. */
export interface Envelope<P = Record<string, unknown>> {
  v: ProtocolVersion;
  id: string;
  method: SdcpMethod;
  params: P;
  hostId?: string;
}

/** Exactly one of `result` / `error` — the schema enforces it, this union types it. */
export type Response<R = Record<string, unknown>> =
  | { v: ProtocolVersion; id: string; result: R }
  | { v: ProtocolVersion; id: string; error: SdcpError };

/** One-way push. `seq` is monotonic per daemon run; a gap is a replay request, not a lost paint. */
export interface Notification {
  v: ProtocolVersion;
  seq: number;
  /** RFC 3339 UTC, from the daemon's clock. The reducer is pure, so it is never `Date.now()`. */
  ts: string;
  sessionId?: string | null;
  turnId?: string | null;
  event: SdcpEvent;
}

/** Every method name the daemon answers (schema `$defs.methodNames`). */
export type SdcpMethod =
  | 'host.status'
  | 'host.doctor'
  | 'host.add'
  /**
   * Trust a host's key: the answer to the question `host.add` asks (0.7.13).
   *
   * `host.add` scans the machine's host key and, when it is not the one SDC pinned, records the host
   * `untrusted` and puts the fingerprint on the screen. This method pins it - after scanning **again**,
   * so what was confirmed is what is stored - and only then may the password (when the dialog still has
   * it) be spent on the one-time key install. See `docs/REMOTE.md` §4.
   */
  | 'host.trust'
  /**
   * What a host presents **now** (0.7.13) - the read that makes the trust question answerable again.
   *
   * `host.add` asks it once and pushes the answer as a `HostStatus`; a window that was not open at that
   * moment (a relaunch, a second window) has the row and the sentence but not the *fingerprint*, and a
   * button needs a value rather than a paragraph. The same call answers the re-pin case: a host whose key
   * changed replies `matches: false` with the fingerprint it presents now.
   */
  | 'host.key'
  /**
   * The **public** half of the key SDC uses for hosts it adds (0.7.13) - read-only, and it never makes a
   * key. It is here so a surface can show the one line that finishes a host SDC cannot: a machine that
   * requires a verification code is set up by hand, and the line to paste is this key.
   */
  | 'ssh.key'
  | 'host.remove'
  | 'host.shutdown'
  | 'session.open'
  | 'session.close'
  | 'session.list'
  | 'session.update'
  | 'session.fork'
  /** The folder a chat works in (0.7.6). */
  | 'project.add'
  | 'project.list'
  | 'project.remove'
  | 'engine.start'
  | 'engine.cancel'
  | 'engine.kill'
  | 'engine.status'
  | 'engine.switch'
  | 'fs.read'
  | 'fs.write'
  | 'fs.list'
  | 'fs.stat'
  | 'fs.search'
  | 'git.status'
  | 'git.diff'
  | 'git.checkpoint'
  | 'git.worktree'
  | 'pty.open'
  | 'pty.write'
  | 'pty.resize'
  | 'pty.close'
  /** The output tail of a long-running process: what a login flow and a log view both read. */
  | 'pty.output'
  /**
   * Signing a CLI in from the app (spec section 9.10). The daemon drives the CLI's own login, shows
   * the URL it prints, and hands back the code the user pastes - it never sees the credential.
   */
  | 'cli.login'
  | 'cli.login.status'
  | 'cli.login.code'
  | 'cli.login.cancel'
  | 'cli.recipes'
  /**
   * The model catalogue. `refresh` asks each provider's own endpoint and caches the answer; a row's
   * `source` says whether it is `live`, `cache` or `bundled`, so "always up to date" is visible
   * rather than asserted.
   */
  | 'models.list'
  | 'models.select'
  /**
   * One command, run to completion by the daemon: the **execute** step an agent loop needs, and the
   * one a user can drive directly. `ok` is the exit code's story; `error` is the plain-English
   * translation of a failure (spec section 14.9); `timedOut` says the command was stopped.
   */
  | 'shell.run'
  | 'event.list'
  | 'event.append'
  | 'event.subscribe'
  | 'provider.list'
  | 'provider.test'
  | 'provider.save'
  | 'provider.remove'
  | 'provider.oauth.open'
  | 'provider.oauth.callback'
  | 'provider.local.doctor'
  | 'provider.registry.list'
  | 'provider.registry.set'
  | 'checkpoint.create'
  | 'checkpoint.list'
  | 'checkpoint.restore'
  | 'rewind.apply'
  | 'rewind.redo'
  | 'duel.start'
  | 'duel.keep'
  | 'duel.discard'
  | 'permission.request'
  | 'permission.resolve'
  | 'console.attach'
  | 'console.detach';

/**
 * Host lifecycle (schema `$defs.eventTypes` → `HostStatus`).
 *
 * `untrusted` is 0.7.13's fifth state, and it is the one that asks a question: the host answered, its
 * key is one SDC has never seen, and nothing has been sent to it. `HostStatusEvent.hostKey` carries the
 * fingerprint the dialog shows, and `host.trust` is the answer.
 */
export type HostStatusValue = 'connected' | 'untrusted' | 'degraded' | 'offline' | 'connecting';

/** Provider lifecycle — `needs-auth` is the state that lights the topbar dot. */
export type ProviderLifecycle = 'connected' | 'needs-auth' | 'available' | 'error';

export type ProviderKind = 'subscription' | 'api-key' | 'local' | 'custom';

export type EngineStatusValue =
  | 'idle'
  | 'running'
  | 'awaiting'
  | 'stuck'
  | 'done'
  | 'failed'
  | 'killed';

export type PermissionRisk = 'SAFE' | 'MUTATING' | 'DANGEROUS';

export type PermissionDecision = 'allow_once' | 'always_allow' | 'deny' | 'show_me';

export type TierName = 'Fast' | 'Balanced' | 'Deep';

export interface ProviderRecord {
  id: string;
  name: string;
  kind: ProviderKind;
  status: ProviderLifecycle;
  detail?: string;
  account?: string | null;
  logo?: string;
  url?: string | null;
  protocol?: string | null;
  /** The one letter drawn inside the card's logo circle. The daemon sends it; derived if absent. */
  initial?: string;
}

export interface CheckpointRecord {
  id: string;
  turn: number;
  ts: string;
  title?: string;
  /** Path under the daemon data dir, or null when the turn changed nothing renderable. */
  thumbnail?: string | null;
  filesHash: string;
  rewindRef?: number | null;
}

export interface DoctorCheck {
  id: string;
  label: string;
  state: 'ok' | 'warn' | 'fail';
  detail: string;
  /** Present only on warn/fail: the action button the row grows (`Fix`, `Install`, `Kill process`). */
  fix?: string;
}

export interface RegistryModel {
  id: string;
  provider: string;
  tier: 'fast' | 'balanced' | 'deep';
  ctx: number;
  cost: string;
  enabled: boolean;
  size?: string;
}

export interface SessionRecord {
  id: string;
  hostId: string;
  title: string;
  state: 'idle' | 'running' | 'waiting' | 'success' | 'error';
  turnCount: number;
  updatedAt: string;
  /** The folder this chat works in, when it has one (0.7.6). `null`/absent means "no folder yet". */
  projectId?: string | null;
  projectRoot?: string | null;
}

/**
 * A folder a chat can work in (0.7.6) - `project.add` / `project.list`.
 *
 * `chats` is how many sessions are bound to it, which is what `project.remove` reports back and what a
 * sidebar row would count. The row exists in the daemon's schema from the first migration
 * (`projects`, with `sessions.project_id`); 0.7.6 is where the app can create one.
 */
export interface ProjectRecord {
  projectId: string;
  hostId: string;
  /** The absolute path on the host, as the person picked it. */
  root: string;
  /** The last path segment, or a name the daemon was given. */
  name: string;
  chats: number;
}

/**
 * One host as `session.list` reports it: the row `host.status` names, plus the sessions it owns.
 *
 * `sessions` is the part that matters. The daemon's `hosts` table and its `sessions` table are two
 * tables, and a window that only received the host rows would show a host that had lost all of its
 * chats after a restart - which is exactly what a fresh window used to do, because nothing asked for
 * the list at all.
 */
export interface HostRecord {
  hostId: string;
  name: string;
  hostType: 'local' | 'vps';
  status: HostStatusValue;
  platform?: string | null;
  /** The `user@host` the host was added with; null for `local`. Used to refuse a duplicate. */
  target?: string | null;
  /**
   * The port it was added on (0.7.13), or null for the default.
   *
   * This field is the fix for half of the "vps connect korai jasse nah" report: 0.7.0 parsed the port,
   * used it for the first probe and then dropped it, so every later connection to a VPS on 8443 would
   * have gone to 22.
   */
  port?: number | null;
  /**
   * The fingerprint a person **pinned** for this host (0.7.13), or null - which is every host whose key
   * no one has decided about yet, and every `local` host.
   *
   * It is the row's copy of the decision: `HostStatus.hostKey` carries the fingerprint a host is
   * *waiting* to be trusted with, and this is the one that is already stored (`hosts.host_key`, written
   * by `host.trust`). A window that never saw the event still knows what a host's key is.
   */
  hostKey?: string | null;
  sessions: SessionListItem[];
}

/** A session as `session.list` reports it: what the sidebar draws, and nothing more. */
export interface SessionListItem {
  sessionId: string;
  hostId: string;
  title: string;
  prompt: string;
  state: 'idle' | 'running' | 'waiting' | 'success' | 'error';
  unread: number;
  /** The daemon computed this; the reducer may never read a wall clock. */
  minutesAgo: number;
  attention?: 'awaiting_approval' | 'stuck' | 'budget_stop' | null;
  /** The folder this chat works in, when it has one (0.7.6); the engines run there. */
  projectId?: string | null;
  projectRoot?: string | null;
}

export interface FsHit {
  path: string;
  line: number;
  text: string;
}

/**
 * One row of `fs.list` (0.7.7) - what the window's file tree draws.
 *
 * `dir` is why the tree can expand a folder without a `fs.stat` round trip per row, and `path` is
 * absolute so nothing in the app has to join path strings (a join is where a separator goes wrong).
 */
export interface FsEntry {
  name: string;
  path: string;
  dir: boolean;
  size: number;
}

/**
 * The append-only event catalogue (spec section 5.4), as a discriminated union on `type`.
 *
 * These are the events the UI reducer projects (spec section 3.3). The first eighteen are the
 * subset the UI needs; the last five are the differentiator events of spec section 2.5 —
 * `StuckDetected` (12.9), `ConsoleError` (15.4), `DuelStarted`/`DuelResolved` (16.6) and
 * `SessionBridged` (16.5). Every one of them is emitted by the daemon and *never* by the UI; the
 * UI's only way in is `dispatch(intent)` → `sdcp_call` → daemon appends → notification back.
 */

export interface HostStatusEvent {
  type: 'HostStatus';
  hostId: string;
  name: string;
  hostType: 'local' | 'vps';
  status: HostStatusValue;
  /** sdcd version on the far side, e.g. `0.4.1`. */
  sdcd?: string;
  /** Machine line for the tooltip: `macOS 15.1 · arm64`, `Debian 12 · x64`. */
  platform?: string;
  /**
   * What just happened to this host, in words (0.7.13).
   *
   * `platform` is the machine line and this is the *sentence* - `root@vps is reachable`, `copying SDC's
   * key with that password…`, `its host key is not the one SDC pinned for it…`. Until 0.7.13 the
   * daemon had one parameter for both jobs and the sentences travelled in `platform`, so the card that
   * reads the machine line read "copying SDC's key…" instead.
   */
  detail?: string | null;
  /**
   * The fingerprint this host is **waiting to be trusted** with, when `status` is `untrusted`.
   *
   * This is the string `host.trust` takes back, and it is a field rather than a sentence to be parsed
   * because a button needs an exact value.
   */
  hostKey?: string | null;
}

/**
 * `host.remove`'s event: the host is gone from this daemon's list for good.
 *
 * A removed host needs an event rather than a local patch for the same reason `HostStatus` is an
 * event: the host list is the daemon's, so the only way a second window - or this one, after a
 * restart - learns that a host was removed is by folding what the daemon appended. `sessions` is the
 * count that went with it, so the toast can say what was thrown away with it.
 */
export interface HostRemovedEvent {
  type: 'HostRemoved';
  hostId: string;
  name: string;
  sessions: number;
}

export interface ProviderStatusEvent {
  type: 'ProviderStatus';
  id: string;
  name?: string;
  status: ProviderLifecycle;
  detail?: string;
  account?: string | null;
  /** `api-key` flows report the same structured result `provider.test` returns. */
  models?: number;
  /** Presentation: which gradient logo and letter the card draws (spec section 9.10). */
  kind?: ProviderKind;
  logo?: string;
  initial?: string;
  /** Custom endpoints carry the URL and the protocol they speak. */
  url?: string;
  protocol?: string;
}

/** The model registry of spec section 9.10, as one event: `provider.registry.list`'s answer. */
export interface RegistryLoadedEvent {
  type: 'RegistryLoaded';
  models: RegistryModel[];
}

export interface SessionOpenedEvent {
  type: 'SessionOpened';
  sessionId: string;
  hostId: string;
  title: string;
  prompt: string;
  /** The folder the chat was opened on, when `Open folder` created it (0.7.6). */
  projectId?: string | null;
  projectRoot?: string | null;
}

export interface SessionClosedEvent {
  type: 'SessionClosed';
  sessionId: string;
}

export interface SessionUpdatedEvent {
  type: 'SessionUpdated';
  sessionId: string;
  title?: string;
  prompt?: string;
  state?: 'idle' | 'running' | 'waiting' | 'success' | 'error';
  /** The folder this chat works in, when the update was `Open folder` on an existing chat (0.7.6). */
  projectId?: string | null;
  projectRoot?: string | null;
  /** Unread turns; a non-zero count is the blue sidebar badge. */
  unread?: number;
  /**
   * Age in minutes, as the daemon computed it. It travels *in the event* rather than being
   * subtracted from a wall clock by the reducer, which is what keeps the reducer pure.
   */
  minutesAgo?: number;
  attention?: 'awaiting_approval' | 'stuck' | 'budget_stop' | null;
}

export interface TurnStartedEvent {
  type: 'TurnStarted';
  turnId: string;
  sessionId: string;
  engine: string;
  model: string;
  tier: TierName;
  /** What the user asked, verbatim. The log carries it so a reloaded window can draw the question
   *  beside the answer instead of showing a conversation that starts with the reply. */
  prompt: string;
}

export interface TurnDeltaEvent {
  type: 'TurnDelta';
  turnId: string;
  /** The text appended to the turn's answer. Streaming areas carry `aria-live="polite"`. */
  delta: string;
}

export interface TurnCompletedEvent {
  type: 'TurnCompleted';
  turnId: string;
  summary: string;
  meta: string;
  pass?: boolean;
}

export interface ToolCallStartedEvent {
  type: 'ToolCallStarted';
  turnId: string;
  callId: string;
  tool: 'read' | 'edit' | 'run';
  name: string;
  target: string;
}

export interface ToolCallOutputEvent {
  type: 'ToolCallOutput';
  turnId: string;
  callId: string;
  level: 'ok' | 'fail' | 'dim';
  text: string;
}

export interface ToolCallCompletedEvent {
  type: 'ToolCallCompleted';
  turnId: string;
  callId: string;
  status: 'done' | 'failed';
  meta: string;
  diff?: { lineNumber: string; text: string; change: 'add' | 'rem' }[];
}

export interface ThinkingDeltaEvent {
  type: 'ThinkingDelta';
  turnId: string;
  delta: string;
}

export interface ErrorRaisedEvent {
  type: 'ErrorRaised';
  sessionId?: string | null;
  turnId?: string | null;
  title: string;
  /** The plain-English translation of spec section 14.9 — never a raw stack trace. */
  explanation: string;
  /** The raw line, kept for `Show code`. */
  source?: string;
  fixable?: boolean;
}

export interface PermissionRequestedEvent {
  type: 'PermissionRequested';
  permissionId: string;
  sessionId: string;
  turnId?: string | null;
  title: string;
  sub: string;
  action: string;
  target: string;
  risk: PermissionRisk;
  explain?: string;
  checkpointId?: string | null;
}

export interface PermissionResolvedEvent {
  type: 'PermissionResolved';
  permissionId: string;
  decision: PermissionDecision;
}

export interface CheckpointSavedEvent {
  type: 'CheckpointSaved';
  sessionId: string;
  checkpoint: CheckpointRecord;
}

export interface RewindAppliedEvent {
  type: 'RewindApplied';
  sessionId: string;
  direction: 'back' | 'forward';
  turn: number;
  /** Turns dropped (back) or re-applied (forward). */
  turns: number;
  files: number;
}

export interface ToastEvent {
  type: 'Toast';
  message: string;
  action?: string;
  /** Overrides the 3s default of spec section 9.14 (`Undo this` holds for 10s). */
  holdMs?: number;
}

/**
 * The toast has been shown for its hold and is going away (spec section 9.14's "3s hold, fade").
 *
 * It is an event rather than a component-local timer because the log is the only place a fact may
 * live: without this, the stack would be the second owner of "which toasts exist", which is exactly
 * what spec section 3.3 rules out.
 */
export interface ToastDismissedEvent {
  type: 'ToastDismissed';
  id: number;
}

/** Spec section 12.9: a turn that has produced nothing for the timeout window. */
export interface StuckDetectedEvent {
  type: 'StuckDetected';
  turnId: string;
  sessionId: string;
  sinceMs: number;
}

/** Spec section 15.4: a preview console line, ready for `Fix with agent`. */
export interface ConsoleErrorEvent {
  type: 'ConsoleError';
  sessionId: string;
  level: 'error' | 'warn' | 'info';
  message: string;
  source: string;
  file: string;
  line: number;
}

/** One engine's run inside a duel (spec section 16.6). */
export interface DuelPane {
  engine: string;
  model: string;
  /** Wall-clock time the run took, as the daemon measured it: `48s`. */
  time: string;
  cost: string;
  pass: boolean;
  headline: string;
  files: string[];
}

/** Spec section 16.6: both engines are running the same prompt. */
export interface DuelStartedEvent {
  type: 'DuelStarted';
  duelId: string;
  sessionId: string;
  prompt: string;
  engines: string[];
  /** The two panes, once both runs have produced a result. */
  panes?: DuelPane[];
}

export interface DuelResolvedEvent {
  type: 'DuelResolved';
  duelId: string;
  /** The engine that was kept, or null for `Keep neither` — both are archived, not deleted. */
  kept: string | null;
}

/** Spec section 16.5: a mid-turn engine switch that carried the conversation across. */
export interface SessionBridgedEvent {
  type: 'SessionBridged';
  sessionId: string;
  turnId: string;
  from: string;
  to: string;
  model: string;
  reason?: string;
}

export type SdcpEvent =
  | HostStatusEvent
  | HostRemovedEvent
  | ProviderStatusEvent
  | RegistryLoadedEvent
  | SessionOpenedEvent
  | SessionClosedEvent
  | SessionUpdatedEvent
  | TurnStartedEvent
  | TurnDeltaEvent
  | TurnCompletedEvent
  | ToolCallStartedEvent
  | ToolCallOutputEvent
  | ToolCallCompletedEvent
  | ThinkingDeltaEvent
  | ErrorRaisedEvent
  | PermissionRequestedEvent
  | PermissionResolvedEvent
  | CheckpointSavedEvent
  | RewindAppliedEvent
  | ToastEvent
  | ToastDismissedEvent
  | StuckDetectedEvent
  | ConsoleErrorEvent
  | DuelStartedEvent
  | DuelResolvedEvent
  | SessionBridgedEvent;

/** The `type` literals, in catalogue order — used by tests and by the reducer's exhaustiveness. */
export const SDCP_EVENT_TYPES = [
  'HostStatus',
  'HostRemoved',
  'ProviderStatus',
  'RegistryLoaded',
  'SessionOpened',
  'SessionClosed',
  'SessionUpdated',
  'TurnStarted',
  'TurnDelta',
  'TurnCompleted',
  'ToolCallStarted',
  'ToolCallOutput',
  'ToolCallCompleted',
  'ThinkingDelta',
  'ErrorRaised',
  'PermissionRequested',
  'PermissionResolved',
  'CheckpointSaved',
  'RewindApplied',
  'Toast',
  'ToastDismissed',
  'StuckDetected',
  'ConsoleError',
  'DuelStarted',
  'DuelResolved',
  'SessionBridged',
] as const satisfies readonly SdcpEvent['type'][];

/** Every event as a `Record` keyed by `type`, handy for a switch's exhaustiveness check. */
export type SdcpEventByType = {
  [K in SdcpEvent['type']]: Extract<SdcpEvent, { type: K }>;
};

/**
 * Method contracts — the `methods` block of the schema, in TypeScript.
 *
 * `SdcpMethodMap` is what makes `sdcpCall()` typed at the call site: the params object is checked
 * against the method's `params`, and the resolved value against its `result`. Methods the UI never
 * calls are present with an empty shape rather than omitted, so the map stays a faithful mirror of
 * `envelope.method`'s enum: adding a method to the schema and forgetting the map is a compile error
 * in `lib/sdcp.ts`, which is the point.
 */
export interface SdcpMethodMap {
  'host.status': { params: Record<string, never>; result: HostStatusEvent };
  'host.doctor': { params: { hostId?: string; sessionId?: string }; result: { checks: DoctorCheck[] } };
  'host.add': {
    params: {
      type: 'local' | 'ssh';
      /**
       * What the user typed: `user@host`, or the whole `ssh -p 8443 user@host` command they run in a
       * terminal. The daemon parses the address and the port out of it (`auth::remote::parse_target`).
       */
      target?: string;
      label?: string;
      /**
       * The password for a host that asks for one, when the user chooses to give it.
       *
       * It is used for a single `ssh` call - the one that copies SDC's public key into
       * `~/.ssh/authorized_keys` - and is stored nowhere. See `auth::remote::install_key`.
       */
      password?: string;
    };
    /** `reused` is true when that `user@host` was already in the list - the row is returned as-is. */
    result: { hostId: string; reused: boolean };
  };
  /**
   * The answer to the trust question `host.add` may have asked (0.7.13).
   *
   * `fingerprint` is the string the dialog showed (`SHA256:…`), and it is re-checked against the
   * machine before anything is written: a key that changed between the question and the answer is
   * refused rather than pinned. `password` is only spent **after** the pin, which is what makes the
   * one-time key install safe on a host whose identity was never checked.
   */
  'host.trust': {
    params: { hostId: string; fingerprint: string; password?: string };
    result: { trusted: boolean; hostId: string; fingerprint: string };
  };
  /**
   * The scan `host.add` does once, as a call (0.7.13).
   *
   * `matches` is `null` for a host SDC has no pin for (nothing to match against), `true` when the key is
   * the pinned one, and `false` when it is **not** - which is the case a `Re-pin` button confirms and the
   * case nothing in this daemon offers to "continue anyway" through.
   */
  'host.key': {
    params: { hostId: string };
    result: {
      hostId: string;
      hostKey: string;
      keyType: string;
      pinned: boolean;
      matches: boolean | null;
      pinnedKey: string | null;
    };
  };
  /**
   * SDC's own **public** key (`~/.ssh/sdc_ed25519.pub`), for the surfaces that have to show it - the
   * `authorized_keys` line a person pastes when a host requires a verification code (0.7.13).
   */
  'ssh.key': {
    params: Record<string, never>;
    result: { publicKey: string | null; path: string | null; exists: boolean };
  };
  /** The row, its sessions and their turns go. `local` is refused: it is the machine `sdcd` runs on. */
  'host.remove': {
    params: { hostId: string };
    result: { removed: boolean; name: string; sessions: number };
  };
  /** Ends the daemon after this request. The app uses it to replace a daemon of another version. */
  'host.shutdown': {
    params: Record<string, never>;
    result: { stopping: boolean; sdcd: string; clients: number };
  };

  'session.open': {
    params: { hostId: string; title?: string; prompt?: string; projectId?: string };
    result: { sessionId: string; projectId?: string | null; projectRoot?: string | null };
  };
  'session.close': { params: { sessionId: string }; result: Record<string, never> };
  'session.list': { params: Record<string, never>; result: { hosts: HostRecord[] } };
  'session.update': {
    params: { sessionId: string; title?: string; state?: SessionRecord['state']; projectId?: string };
    result: { projectId?: string | null; projectRoot?: string | null };
  };
  'session.fork': {
    params: { sessionId: string; atTurn?: number };
    result: { sessionId: string; /** How many turns came with the fork. */ turns: number; title: string };
  };

  'project.add': {
    params: { hostId?: string; root: string; name?: string };
    result: { projectId: string; hostId: string; root: string; name: string };
  };
  'project.list': { params: Record<string, never>; result: { projects: ProjectRecord[] } };
  'project.remove': { params: { projectId: string }; result: { removed: boolean; chats: number } };

  'engine.start': {
    params: {
      sessionId: string;
      prompt: string;
      engine: string;
      model: string;
      tier: TierName;
      /**
       * The provider the model was picked from, when the app knows it (`deepseek`).
       *
       * It is optional so the protocol stays compatible, and load-bearing when present: a provider's
       * live list contains model ids this build's catalogue has never seen (`deepseek-v4-pro`), and
       * without the provider the daemon cannot tell which endpoint - and which key - such an id belongs
       * to. The turn then fails with `No API key for custom` for a provider that is connected.
       */
      provider?: string;
    };
    result: { turnId: string };
  };
  'engine.cancel': { params: { turnId: string }; result: Record<string, never> };
  'engine.kill': { params: { turnId: string }; result: Record<string, never> };
  'engine.status': {
    params: { turnId: string };
    result: { state: EngineStatusValue; engine: string; model: string };
  };
  'engine.switch': {
    params: { turnId: string; engine: string; model: string; reason?: string };
    result: { turnId: string; bridgedFrom: string };
  };

  'fs.read': {
    /** `hostId` names the machine the path is on (0.7.13); absent means this one. */
    params: { path: string; hostId?: string };
    result: {
      path: string;
      text: string;
      sha256: string;
      /** The file's real size, even when `text` was cut. */
      bytes: number;
      /** True when the file is larger than the daemon's cap, so `text` is its first megabyte. */
      truncated: boolean;
    };
  };
  'fs.write': {
    params: { path: string; text: string; sessionId?: string; turnId?: string; hostId?: string };
    /**
     * `bytes` is what was written. There is no `checkpointId` here: the checkpoint a Save takes (P5) arrives as
     * a `CheckpointSaved` event, which is where the Time Machine reads it from - and the field this type used to
     * declare was one the daemon never answered.
     */
    result: { path: string; sha256: string; bytes: number };
  };
  'fs.list': {
    /**
     * `path` may be omitted since 0.7.7: the **session's** folder is listed then.
     *
     * Since 0.7.13 `hostId` may name another machine, and the same answer comes back from it - absolute
     * paths, the guard's hidden count, one level. The app sends the chat's host so a tree on a VPS is
     * the same tree the sidebar already draws.
     */
    params: { path?: string; sessionId?: string; hostId?: string };
    result: { path: string; entries: FsEntry[]; hidden: number };
  };
  'fs.stat': { params: { path: string; hostId?: string }; result: { size: number; sha256: string } };
  'fs.search': {
    params: { query: string; glob?: string; root?: string; sessionId?: string; hostId?: string };
    result: { hits: FsHit[] };
  };

  'git.status': {
    params: { sessionId?: string; root?: string; hostId?: string };
    result: { branch: string; dirty: number };
  };
  'git.diff': {
    /** Either a session (whose folder is used) or a `root`; the daemon refuses both-missing in words. */
    params: { sessionId?: string; root?: string; checkpointId?: string; sha?: string; hostId?: string };
    result: { patch: string };
  };
  'git.checkpoint': {
    params: { sessionId: string; turnId?: string };
    result: { checkpointId: string; sha: string };
  };
  'git.worktree': { params: { sessionId: string }; result: { path: string } };

  /**
   * `pty.open` starts a long-running process, **here or on a host** (0.7.13).
   *
   * With `hostId` the daemon's child is an `ssh` and the process runs on that machine: `pty.output`
   * reads its output tail, `pty.write` reaches its stdin, and `pty.close` signals its **process group**
   * there. `tty` is false either way - there is no pty, so a full-screen program is not this.
   *
   * `line` is a whole command as a person typed it (what the Terminal's `Run in background` sends) and
   * `command` + `args` is a program SDC already knows (`cli.login`). With `line`, the local platform's
   * shell runs it here and the **host's** shell runs it there - and the line is checked against the deny
   * list before it starts, statement by statement. One of the two forms is required.
   */
  'pty.open': {
    params: {
      command?: string;
      args?: string[];
      line?: string;
      cwd?: string;
      sessionId?: string;
      hostId?: string;
    };
    result: { ptyId: string; command: string; tty: boolean; hostId?: string };
  };
  'pty.write': { params: { ptyId: string; data: string }; result: Record<string, never> };
  'pty.resize': {
    params: { ptyId: string; cols: number; rows: number };
    result: Record<string, never>;
  };
  'pty.close': { params: { ptyId: string }; result: Record<string, never> };
  'pty.output': {
    params: { ptyId: string };
    result: {
      ptyId: string;
      command: string;
      state: 'running' | 'exited' | 'gone';
      lines: string[];
      lineCount: number;
      ms: number;
    };
  };

  /**
   * `cli.login` starts the CLI's own sign-in. The URL is not in this answer - it arrives in
   * `cli.login.status` a moment later, because a CLI prints it once its screen is drawn.
   * `program`/`args`/`pump` override the recipe, which is how the flow is tested without a CLI.
   */
  'cli.login': {
    params: {
      providerId: string;
      program?: string;
      args?: string[];
      pump?: string[];
    };
    result: { loginId: string; ptyId: string; program: string; providerId: string };
  };
  'cli.login.status': {
    params: { loginId: string };
    result: {
      loginId: string;
      providerId: string;
      providerLabel: string;
      program: string;
      /** The page to approve, once the CLI has printed it. */
      url: string | null;
      state: 'starting' | 'waiting_for_url' | 'waiting_for_code' | 'authenticated' | 'exited' | 'failed' | 'cancelled';
      /** What this recipe expects the user to know, from the daemon's own table. */
      note: string | null;
      /** The CLI's output tail, so a recipe that goes stale is visible instead of silent. */
      lines: string[];
      lineCount: number;
      ms: number;
      authenticated: boolean;
    };
  };
  'cli.login.code': {
    params: { loginId: string; code: string };
    result: { submitted: boolean; loginId: string };
  };
  'cli.login.cancel': {
    params: { loginId: string };
    result: { cancelled: boolean; loginId: string };
  };
  'cli.recipes': {
    params: Record<string, never>;
    result: {
      recipes: { providerId: string; label: string; program: string; note: string; installed: boolean }[];
    };
  };

  'models.list': {
    params: { providerId?: string; refresh?: boolean };
    result: {
      models: {
        id: string;
        /**
         * The name to show a person: the bundle's own where it has one (`Claude Sonnet 4.5`), the
         * daemon's spelling of the id where the provider sent only an id.
         */
        name: string;
        providerId: string;
        providerLabel: string;
        tier: TierName;
        ctx: number;
        cost: string;
        /** Where the row came from: the provider just now, the last live answer, or the bundle. */
        source: 'live' | 'cache' | 'bundled';
        fetchedAt?: string | null;
      }[];
      /** The day the bundled catalogue was last curated. */
      snapshot: string;
      refreshed: boolean;
      /** Why a refresh could not reach a provider, in words. Empty when every one was reached. */
      notes: string[];
      selected: { modelId: string | null; providerId: string | null };
    };
  };
  'models.select': {
    params: { modelId: string; providerId?: string };
    result: { modelId: string; providerId?: string | null };
  };

  /**
   * One command, run to completion. The daemon writes a checkpoint before it starts (the command may
   * mutate the tree) and reports the failure in words, never as a stack trace.
   */
  'shell.run': {
    params: {
      /** A program with `args` - **or** `line`, which is the whole command as a person typed it. */
      command?: string;
      args?: string[];
      /**
       * A whole command line, run by the platform's own shell (`sh -c` here, `cmd /C` on Windows, the
       * remote shell on a host) - what the Terminal tab sends (0.7.13). Every *statement* of it is
       * checked against the deny list, so `git status && shutdown /s` is refused like a bare
       * `shutdown /s` would be. One of `command` and `line` is required.
       */
      line?: string;
      cwd?: string;
      /** Given a session and a `root`, a checkpoint is written before the command runs. */
      sessionId?: string;
      turnId?: string;
      root?: string;
      /**
       * The machine the command runs on (0.7.13). Given a host, `cwd` and `command` are the *host's*
       * folder and program, the deny list still applies, and the answer keeps the same shape.
       */
      hostId?: string;
      /** How long the command may run; the default is 120 s. */
      timeoutMs?: number;
    };
    result: {
      command: string;
      args: string[];
      cwd: string | null;
      exitCode: number | null;
      ok: boolean;
      stdout: string;
      stderr: string;
      durationMs: number;
      timedOut: boolean;
      truncated: boolean;
      error: { title: string; explanation: string; rule: string; fixable: boolean } | null;
    };
  };

  'event.list': {
    params: { since?: number; sessionId?: string };
    result: { events: Notification[] };
  };
  'event.append': { params: { event: SdcpEvent }; result: { seq: number } };
  'event.subscribe': { params: { since?: number }; result: { fromSeq: number } };

  'provider.list': { params: Record<string, never>; result: { providers: ProviderRecord[] } };
  /**
   * Flow 1's `Test`.
   *
   * `ok` means "the check ran and the answer was good"; `verified` says whether the **provider** was
   * actually contacted. A key's shape is checked locally; only the local Ollama daemon and an
   * `http://` endpoint can be reached in this build, so a saved key answers `verified: false` with a
   * `detail` that spells it out - a green tick that means "we never asked the provider" would be
   * worse than no tick at all.
   */
  'provider.test': {
    params: { id: string; key?: string };
    result: { ok: boolean; models: number; detail: string; verified: boolean; error?: string | null };
  };
  'provider.save': {
    params: {
      id: string;
      kind: ProviderKind;
      key?: string;
      label?: string;
      url?: string;
      protocol?: string;
    };
    result: { id: string; status: ProviderLifecycle; account?: string };
  };
  'provider.remove': { params: { id: string }; result: { removed: boolean } };
  'provider.oauth.open': { params: { id: string }; result: { url: string; state: string } };
  'provider.oauth.callback': {
    params: { id: string; state: string; code?: string };
    result: { ok: boolean; account?: string };
  };
  'provider.local.doctor': {
    params: Record<string, never>;
    result: { daemon: boolean; endpoint: string; models: string[] };
  };
  'provider.registry.list': {
    params: Record<string, never>;
    result: { models: RegistryModel[] };
  };
  'provider.registry.set': {
    params: { id: string; enabled: boolean };
    result: Record<string, never>;
  };

  'checkpoint.create': {
    params: { sessionId: string; turnId: string; title: string };
    result: { checkpointId: string };
  };
  'checkpoint.list': {
    params: { sessionId: string };
    result: { checkpoints: CheckpointRecord[] };
  };
  'checkpoint.restore': { params: { checkpointId: string }; result: { restored: number } };

  'rewind.apply': {
    params: { sessionId: string; turnId: string };
    result: { removedTurns: number; restoredFiles: number };
  };
  'rewind.redo': { params: { sessionId: string }; result: { turn: number | null } };

  'duel.start': {
    params: { sessionId: string; prompt: string; engines: string[] };
    result: { duelId: string };
  };
  'duel.keep': { params: { duelId: string; keep: string }; result: Record<string, never> };
  'duel.discard': { params: { duelId: string }; result: Record<string, never> };

  'permission.request': {
    params: {
      sessionId: string;
      turnId: string;
      action: string;
      target: string;
      risk: PermissionRisk;
    };
    result: { permissionId: string };
  };
  'permission.resolve': {
    params: { permissionId: string; decision: PermissionDecision; scope?: string };
    result: Record<string, never>;
  };

  'console.attach': { params: { sessionId: string; url: string }; result: { attached: boolean } };
  'console.detach': { params: { sessionId: string }; result: { detached: boolean } };
}

export type MethodParams<M extends SdcpMethod> = SdcpMethodMap[M]['params'];
export type MethodResult<M extends SdcpMethod> = SdcpMethodMap[M]['result'];

/**
 * A transport carries envelopes out and responses plus notifications back (spec section 3.1).
 *
 * `lib/transport.ts` has three implementations - unix socket, Windows named pipe, and a
 * WebSocket for the remote case - and they are interchangeable: only the pipe differs, never the
 * envelope. The interface is here rather than in `lib/` so the protocol's shape stays in the
 * protocol folder.
 */
export interface SdcpTransport {
  /** `unix` = socket on Linux/macOS, `pipe` = named pipe on Windows, `ws` = remote over SSH. */
  readonly kind: 'unix' | 'pipe' | 'ws';
  send(envelope: Envelope): void;
  /** Returns an unsubscribe function. */
  subscribe(handler: (notification: Notification) => void): () => void;
  /** Rejects with `SdcpError` when the daemon answers with `error`. */
  request<M extends SdcpMethod>(method: M, params: MethodParams<M>): Promise<MethodResult<M>>;
  close(): void;
}

