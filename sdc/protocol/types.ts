/**
 * SDCP 0.1 â€” TypeScript mirror of `protocol/sdcp.schema.json` (spec section 5).
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

/** Exactly one of `result` / `error` â€” the schema enforces it, this union types it. */
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
   * it) be spent on the one-time key install. See `docs/REMOTE.md` Â§4.
   */
  | 'host.trust'
  /**
   * Measure a host again, and say the result on its own row (0.11.3).
   *
   * The window's `Reconnect` button had nothing behind it: it said `Reconnected` and measured nothing, so
   * a host that had come back stayed `offline` until the app was relaunched. This is the probe that
   * answered the *first* question (`host.add`), asked again on demand - and it is the probe alone, because
   * a reconnect has no password to spend and no key to install.
   */
  | 'host.probe'
  /**
   * What a host presents **now** (0.7.13) - the read that makes the trust question answerable again.
   *
   * `host.add` asks it once and pushes the answer as a `HostStatus`; a window that was not open at that
   * moment (a relaunch, a second window) has the row and the sentence but not the *fingerprint*, and a
   * button needs a value rather than a paragraph. The same call answers the re-pin case: a host whose key
   * changed replies `matches: false` with the fingerprint it presents now.
   */
  | 'host.key'
  | 'host.password'
  | 'preview.forward'
  | 'preview.open'
  | 'preview.status'
  | 'preview.dev'
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
  | 'project.scaffold'
  | 'project.locate'
  | 'project.list'
  | 'project.remove'
  | 'engine.start'
  | 'engine.cancel'
  | 'engine.kill'
  | 'engine.steer'
  | 'engine.status'
  | 'engine.switch'
  | 'verify.run'
  | 'fs.read'
  | 'fs.write'
  | 'fs.list'
  | 'fs.stat'
  | 'fs.search'
  | 'fs.rename'
  | 'fs.delete'
  | 'fs.mkdir'
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
  | 'console.detach'
  /* 0.12: the Trust Kernel, the Intent Engine, the agency layer. */
  | 'audit.list'
  | 'audit.verify'
  | 'policy.get'
  | 'policy.set'
  | 'kill.all'
  | 'kill.list'
  /* 0.17: SDC Anywhere - the desktop's settings for the browser feature. */
  | 'anywhere.status'
  | 'anywhere.enable'
  | 'anywhere.disable'
  | 'anywhere.configure'
  | 'anywhere.pair.begin'
  | 'anywhere.pair.requests'
  | 'anywhere.pair.confirm'
  | 'anywhere.devices.list'
  | 'anywhere.devices.revoke'
  | 'anywhere.reset'
  | 'cost.summary'
  | 'cost.estimate'
  | 'cost.budget.set'
  | 'trust.score'
  | 'checkpoint.label'
  | 'checkpoint.files'
  | 'checkpoint.fileDiff'
  | 'checkpoint.restoreFile'
  | 'proof.export'
  | 'intent.detect'
  | 'intent.parse'
  | 'intent.confirm'
  | 'intent.compile'
  | 'intent.cancel'
  | 'intent.stats'
  | 'glossary.list'
  | 'glossary.set'
  | 'voice.status'
  | 'voice.transcribe'
  | 'site.list'
  | 'site.detect'
  | 'site.save'
  | 'site.remove'
  | 'deploy.run'
  | 'deploy.list'
  | 'deploy.get'
  | 'deploy.preview'
  | 'deploy.rollback'
  | 'deploy.restoreDb'
  | 'health.check'
  | 'health.history'
  | 'guardian.set'
  | 'approval.list'
  | 'approval.request'
  | 'approval.decide'
  | 'approval.poll'
  | 'xray.scan'
  | 'xray.get'
  | 'shadowdb.run'
  | 'staging.create'
  | 'staging.stop'
  | 'playbook.list'
  | 'playbook.save'
  | 'playbook.remove'
  | 'playbook.run'
  | 'team.get'
  | 'team.set'
  | 'settings.get'
  | 'settings.set'
  | 'update.check'
  /** 0.21: SDC installs and runs what it needs itself - Node, the coding CLIs, Ollama, ripgrep, Git - so no platform needs a terminal. */
  | 'tool.list'
  | 'tool.install'
  | 'tool.status'
  | 'ollama.start'
  | 'ollama.pull'
  | 'host.port.free'
  /** 0.21: opens the provider connection a turn will use while the person is still typing. */
  | 'provider.warm'
  /** 0.22: any OpenAI-compatible model server on this computer or the network - LM Studio, llama.cpp, vLLM, Jan… */
  | 'local.discover'
  | 'local.add'
  | 'local.remove'
  | 'crash.list'
  | 'crash.clear'
  | 'cli.selfcheck'
  | 'status.share'
  | 'timeline.branches'
  | 'timeline.switch'
  | 'question.answer'
  | 'memory.get'
  | 'memory.set'
  | 'memory.add'
  | 'commands.list'
  | 'files.find'
  | 'process.list'
  | 'process.stop'
  | 'context.get'
  | 'app.erase'
  | 'research.status'
  | 'research.key.set'
  | 'research.plan';

/**
 * Host lifecycle (schema `$defs.eventTypes` â†’ `HostStatus`).
 *
 * `untrusted` is 0.7.13's fifth state, and it is the one that asks a question: the host answered, its
 * key is one SDC has never seen, and nothing has been sent to it. `HostStatusEvent.hostKey` carries the
 * fingerprint the dialog shows, and `host.trust` is the answer.
 */
export type HostStatusValue = 'connected' | 'untrusted' | 'degraded' | 'offline' | 'connecting';

/** Provider lifecycle â€” `needs-auth` is the state that lights the topbar dot. */
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
  /** A name the person gave it (0.12): `Before deploy`. */
  label?: string | null;
  /** Why a rewind to it cannot undo everything after it - a push, a deploy, a database command (0.12). */
  irreversible?: string | null;
  /** The turn that wrote it, when a turn did (0.12). */
  turnId?: string | null;
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

/** One name match of `fs.search` (0.11.0): a file or folder whose name contains the query. */
export interface FsFound {
  path: string;
  dir: boolean;
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
 * subset the UI needs; the last five are the differentiator events of spec section 2.5 â€”
 * `StuckDetected` (12.9), `ConsoleError` (15.4), `DuelStarted`/`DuelResolved` (16.6) and
 * `SessionBridged` (16.5). Every one of them is emitted by the daemon and *never* by the UI; the
 * UI's only way in is `dispatch(intent)` â†’ `sdcp_call` â†’ daemon appends â†’ notification back.
 */

export interface HostStatusEvent {
  type: 'HostStatus';
  hostId: string;
  name: string;
  hostType: 'local' | 'vps';
  status: HostStatusValue;
  /** sdcd version on the far side, e.g. `0.4.1`. */
  sdcd?: string;
  /** Machine line for the tooltip: `macOS 15.1 Â· arm64`, `Debian 12 Â· x64`. */
  platform?: string;
  /**
   * What just happened to this host, in words (0.7.13).
   *
   * `platform` is the machine line and this is the *sentence* - `root@vps is reachable`, `copying SDC's
   * key with that passwordâ€¦`, `its host key is not the one SDC pinned for itâ€¦`. Until 0.7.13 the
   * daemon had one parameter for both jobs and the sentences travelled in `platform`, so the card that
   * reads the machine line read "copying SDC's keyâ€¦" instead.
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
  /** How SDC read the message (0.11.8): its language, and the language the answer comes back in. Absent
   *  when the message went to the engine exactly as typed. */
  reading?: { code: string; label: string; reply: string };
  /** What the turn was estimated to cost before it ran (0.12) - an estimate, labelled as one. */
  estimate?: CostEstimate;
  /** The confirmed Intent Contract the turn was compiled from (0.12). */
  intentId?: string;
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

/**
 * A tool call the model is still writing (0.14.2), before its `ToolCallStarted`: the tool, its target once
 * that much of the input has arrived, how many characters of its body so far, and the newest lines of it.
 * Several arrive per call, a few a second; the newest one is the whole truth.
 */
export interface ToolCallDraftingEvent {
  type: 'ToolCallDrafting';
  turnId: string;
  callId: string;
  name: string;
  target: string;
  chars: number;
  preview: string;
}

export interface ToolCallCompletedEvent {
  type: 'ToolCallCompleted';
  turnId: string;
  callId: string;
  status: 'done' | 'failed';
  meta: string;
  diff?: { lineNumber: string; text: string; change: 'add' | 'rem' }[];
}

/** One row of a verify run: a check the project's own manifests promise. */
export interface VerifyCheck {
  name: string;
  command: string;
  status: 'pending' | 'running' | 'pass' | 'fail';
  ms: number | null;
  /** The last lines of its output. */
  tail: string[];
}

/** One issue a reviewer found, pinned to a place in the change. */
export interface VerifyIssue {
  file: string;
  line: number | null;
  severity: 'high' | 'medium' | 'low';
  message: string;
  fix: string;
}

/** The review stage: which engine read the diff, and what it said. */
export interface VerifyReview {
  engine: string;
  model: string;
  status: 'pending' | 'running' | 'done' | 'skipped' | 'failed';
  verdict?: 'pass' | 'issues' | 'unreadable' | null;
  summary?: string;
  issues?: VerifyIssue[];
  /** The confirmed Intent Contract's conditions, judged one by one (0.12). */
  criteria?: { met: boolean | null; why: string }[];
}

/**
 * The model catalogue moved (0.9.0): a provider's live list was fetched and cached - on a schedule, or
 * because a key was just saved. The rows are NOT in the event (OpenRouter alone lists hundreds); a
 * window that cares asks `models.list`, which now answers from the fresh cache.
 */
export interface ModelsUpdatedEvent {
  type: 'ModelsUpdated';
  /** The providers whose lists were refreshed live, by id. */
  providers: string[];
  /** How many models the catalogue holds across them, after the refresh. */
  models: number;
}

/** A verify run (v4), whole each time: the checks, then the review by a different engine. */
export interface VerifyUpdatedEvent {
  type: 'VerifyUpdated';
  verifyId: string;
  sessionId: string;
  turnId?: string | null;
  state: 'running' | 'done';
  /** `null` until the run is over. */
  pass: boolean | null;
  checks: VerifyCheck[];
  review: VerifyReview | null;
  /** A sentence about the run as a whole, e.g. why no checks were found. */
  note: string;
  /** The secret, SAST and dependency scans of the change (0.12). */
  scans?: VerifyScans;
  /** The run's word (0.12): `UNPROVEN` when nothing failed but nothing proved it either. */
  verdict?: VerifyVerdict | null;
}

/** The agent's checklist for a turn (v4), whole each time: the newest one replaces the last. */
/** Research limits (0.16.1). */
export interface ResearchLimits {
  maxSearches: number;
  maxPages: number;
  maxMinutes: number;
}

export interface ResearchStatus {
  provider: 'duckduckgo' | 'searxng' | 'tavily' | 'brave' | 'serper';
  providerLabel: string;
  searxngUrl: string;
  keys: { provider: 'tavily' | 'brave' | 'serper'; hasKey: boolean; masked: string | null }[];
  limits: ResearchLimits;
  localWebOnly: boolean;
  synthesis: { provider: string; model: string } | null;
  ollamaContext: number;
  ollamaRunning: boolean;
  ollamaModels: string[];
}

export interface ResearchPlan {
  place: 'local' | 'api' | 'cli';
  engine: string;
  model: string;
  provider: string | null;
  search: string;
  searchKeyMissing: boolean;
  limits: ResearchLimits;
  synthesis: { provider: string; model: string } | null;
  localContext: number | null;
  ollamaRunning: boolean | null;
  estimate: { inputTokens: number; outputTokens: number; usd: number | null; source: string; synthesisUsd?: number | null };
}

/** One numbered source of a research answer. */
export interface ResearchSource {
  n: number;
  title: string;
  url: string;
  date: string | null;
  /** Read in full, not only seen in a result list. */
  read: boolean;
}

/** A `/research` turn's sources (0.16.1): the list under its answer, cited in it as [n]. */
export interface ResearchSourcesEvent {
  type: 'ResearchSources';
  turnId: string;
  sessionId: string;
  sources: ResearchSource[];
}

export interface PlanUpdatedEvent {
  type: 'PlanUpdated';
  turnId: string;
  steps: { text: string; status: 'pending' | 'in_progress' | 'done' }[];
}

/** Words the person sent into a running turn, as the model received them (0.12.5). */
export interface TurnSteeredEvent {
  type: 'TurnSteered';
  turnId: string;
  text: string;
}

/**
 * How full the model's context is for a turn (0.13): what it is sent against what the model holds.
 * `compacted` - older turns (or older tool output) were folded; `resumed` - a CLI continues its own
 * conversation, so it holds more than SDC sends.
 */
export interface ContextUpdatedEvent {
  type: 'ContextUpdated';
  sessionId: string;
  turnId: string;
  usedTokens: number;
  windowTokens: number;
  percent: number;
  compacted: boolean;
  resumed: boolean;
}

/** The agent asks the person something and waits (0.13, `ask_user`); `question.answer` replies. */
export interface QuestionAskedEvent {
  type: 'QuestionAsked';
  sessionId: string;
  turnId: string;
  questionId: string;
  question: string;
  options: string[];
}

/** A question was answered - its card closes. */
export interface QuestionAnsweredEvent {
  type: 'QuestionAnswered';
  turnId: string;
  questionId: string;
  answer: string;
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
  /** The plain-English translation of spec section 14.9 â€” never a raw stack trace. */
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
  /** The engine that was kept, or null for `Keep neither` â€” both are archived, not deleted. */
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

/* ================================================================================================
 * The Trust Kernel, the Intent Engine and the agency layer (0.12, docs/MASTER-PLAN-v3-TRUST-KERNEL.md)
 * ============================================================================================== */

/** Where a turn's cost number came from - a number is measured or it says it is not (P4). */
export type CostSource = 'measured' | 'priced' | 'local' | 'subscription' | 'unpriced' | 'none';

/** What a turn will probably cost, before it runs - always labelled an estimate. */
export interface CostEstimate {
  inputTokens: number;
  outputTokens: number;
  usd: number | null;
  source: 'estimate' | 'subscription' | 'unknown';
  complexity: 'simple' | 'normal' | 'complex';
}

export interface CheaperModel {
  provider: string;
  model: string;
  reason: string;
  pricePerMillion: { in: number; out: number };
}

export interface CostSpent {
  day: number;
  month: number;
  chat: number | null;
}

export interface CostBudgets {
  turn?: number | null;
  chat?: number | null;
  day?: number | null;
  month?: number | null;
}

export interface CostSummary {
  days: { day: string; usd: number }[];
  models: { model: string; usd: number; inputTokens: number; outputTokens: number; turns: number }[];
  projects: { root: string; usd: number }[];
  sites: { siteId: string; usd: number }[];
  today: number;
  spent: CostSpent;
  /** Only against a baseline the person chose, and only for measured turns. */
  savedUsd: number;
  measuredTurns: number;
  unmeasuredTurns: number;
  baseline: string | null;
  budgets: CostBudgets;
}

export type TrustLevel = 'high' | 'medium' | 'low';

export interface TrustReason {
  text: string;
  /** Points this reason moved the score by (0 for a reason that only explains). */
  delta: number;
}

/** `.sdc/policy.toml` as the daemon reads it. */
export interface Policy {
  production: boolean;
  maxFilesPerTurn: number;
  privacy: 'any' | 'local-only';
  protectedPaths: string[];
  alwaysAsk: string[];
  denyCommands: string[];
  maxTurnUsd: number | null;
  autoRollback: boolean;
  /** Commands run after the agent writes a file (0.13), `{file}` for its path: `[hooks] after_edit`. */
  afterEdit?: string[];
  source: string;
  error: string | null;
}

/** One row of the hash-chained audit ledger. */
export interface AuditEntry {
  seq: number;
  ts: string;
  sessionId: string | null;
  turnId: string | null;
  actor: string;
  kind: string;
  summary: string;
  detail: unknown;
  prevHash: string;
  hash: string;
}

export interface LedgerCheck {
  intact: boolean;
  entries: number;
  brokenAt: number | null;
  head?: string;
  reason?: string;
}

/** Something the kill switch can stop. */
export interface ActiveWork {
  id: string;
  kind: 'turn' | 'verify' | 'deploy' | 'playbook' | 'guardian';
  sessionId: string | null;
  label: string;
  started: string;
}

/** The Intent Engine's offline first look at a message. */
export interface Detection {
  code: string;
  dialect: string | null;
  script: string;
  romanized: boolean;
  mixed: boolean;
  confidence: number;
  label: string;
  replyIn: string;
  reply: string;
}

/** What SDC understood a request to be - the Intent Contract card's content. */
export interface TaskSpec {
  language: Partial<Detection> & { code: string; label: string };
  kind: string;
  target: { value: string; confidence: number };
  goal: { value: string; confidence: number };
  acceptance: { text: string; checked: boolean }[];
  acceptanceConfidence: number;
  outOfScope: string[];
  risk: 'low' | 'medium' | 'high';
  questions: string[];
  summary: string;
  backTranslation: string;
  showBackTranslation: boolean;
  unsure: ('target' | 'goal' | 'acceptance')[];
  confidence: number;
  glossary: { term: string; meaning: string }[];
  source: 'model' | 'heuristic' | 'person';
}

export interface GlossaryTerm {
  term: string;
  meaning: string;
  scope: string;
}

/** A finding of the secret scanner or the SAST rules, pinned to an added line. */
export interface ScanFinding {
  rule: string;
  severity: 'high' | 'medium' | 'low';
  file: string;
  line: number;
  message: string;
  fix: string;
}

export interface VerifyScans {
  state: 'pending' | 'running' | 'done';
  secrets?: ScanFinding[];
  sast?: ScanFinding[];
  rules?: { secrets: number; sast: number };
  dependencies?: {
    status: 'done' | 'unavailable' | 'none';
    tool?: string;
    counts?: { critical: number; high: number; moderate: number; low: number };
    detail?: string;
  } | null;
}

export type VerifyVerdict = 'PASS' | 'FAIL' | 'NO_CHECKS' | 'UNPROVEN';

export interface HealthReport {
  ts?: string;
  http: { ok: boolean | null; status?: number | null; ms?: number; detail: string };
  ssl: { days: number | null; expires?: string; detail?: string } | null;
  disk: { percent: number | null; freeMb: number | null } | null;
  backup: { configured: boolean; newest?: string | null; ageHours?: number | null; detail?: string } | null;
  errors: { count: number; file?: string } | null;
  deploy: { id: string; state: DeployState; at: string } | null;
  score?: { score: number; level: OpsLevel; reasons: TrustReason[] };
}

export type OpsLevel = 'healthy' | 'watch' | 'at-risk';

export interface SiteRecord {
  id: string;
  name: string;
  hostId: string;
  root: string;
  url: string;
  config: Record<string, unknown>;
  createdAt: string;
  health?: HealthReport | null;
  lastDeploy?: DeployRecord | null;
}

export type DeployState = 'running' | 'success' | 'failed' | 'rolled_back' | 'rollback_failed' | 'awaiting_approval';

export interface DeployStep {
  id: string;
  name: string;
  command: string | null;
  status: 'pending' | 'running' | 'pass' | 'fail' | 'skipped';
  ms: number | null;
  tail: string[];
}

export interface DeployRecord {
  id: string;
  siteId: string;
  kind: string;
  state: DeployState;
  steps: DeployStep[];
  backup: { stem: string; files: { path: string; bytes?: number } | null; db: { path: string; bytes?: number } | null } | null;
  note: string;
  startedAt: string;
  finishedAt: string | null;
}

export interface ApprovalRecord {
  id: string;
  subject: string;
  kind: string;
  state: 'pending' | 'approved' | 'declined' | 'question' | 'used';
  requestedBy: string;
  decidedBy: string | null;
  note: string;
  token: string | null;
  createdAt: string;
  decidedAt: string | null;
}

export interface XrayRisk {
  level: 'critical' | 'high' | 'medium' | 'low';
  title: string;
  why: string;
  fix: string;
}

export interface XrayMap {
  host: string;
  scannedAt: string;
  os: string;
  arch: string | null;
  uptime: string | null;
  cpus: string | null;
  memoryMb: string | null;
  disks: string[];
  services: string[];
  ports: string[];
  sites: { names: string[]; root: string | null; ssl: boolean; listen?: string[]; server?: string }[];
  wordpress: string[];
  databases: string[];
  docker: string[];
  cron: string[];
  runtimes: string[];
  certificates: { name: string; expires: string; days: number | null }[];
  backups: string[];
  risks: XrayRisk[];
}

export interface Playbook {
  id: string;
  name: string;
  steps: { kind: 'command' | 'prompt'; text: string }[];
}

export interface TeamMember {
  name: string;
  role: 'owner' | 'developer' | 'reviewer' | 'client';
}

export interface TeamState {
  members: TeamMember[];
  current: string | null;
  role: TeamMember['role'];
  roles: TeamMember['role'][];
}

export interface ReleaseInfo {
  tag: string;
  name: string;
  url: string;
  prerelease: boolean;
  publishedAt: string;
  assets: { name: string; url: string }[] | null;
}

export interface UpdateInfo {
  channel: string;
  current: string;
  latest: ReleaseInfo | null;
  updateAvailable: boolean;
  /** The release before the running one - the rollback when an update misbehaves. */
  previous: ReleaseInfo | null;
}

export interface CrashReport {
  file: string;
  report: { version: string; os: string; arch: string; at: string; location: string; message: string; backtrace: string };
  /** A pre-filled GitHub issue: sending a report is always the person's own click. */
  issueUrl: string;
}

export interface CliSelfCheck {
  program: string;
  label: string;
  installed: boolean;
  version: string | null;
  signedIn: boolean;
  sentence: string;
}

export interface TimelineBranch {
  id: number;
  turn: number;
  checkpointId: string;
  pushedAt: string;
  branch: boolean;
  checkpoints: number;
  title: string;
  turns: number;
  hasFiles: boolean;
}

export interface CostUpdatedEvent {
  type: 'CostUpdated';
  sessionId: string;
  turnId: string;
  engine: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  costSource: CostSource;
  estimateUsd: number | null;
  savedUsd: number | null;
}

export interface PolicyViolationEvent {
  type: 'PolicyViolation';
  sessionId: string;
  turnId?: string | null;
  rule: string;
  target: string;
  action: string;
  sentence: string;
}

export interface BudgetStopEvent {
  type: 'BudgetStop';
  sessionId: string;
  turnId: string;
  kind: 'budget' | 'runaway';
  sentence: string;
}

/** Something a browser did through SDC Anywhere: `unlock`, `kill`, `decision.refused`, `device.revoked`, ... */
export interface RemoteActivityEvent {
  type: 'RemoteActivity';
  what: string;
  detail: Record<string, unknown>;
}

/** SDC Anywhere (0.17): where the daemon stands, for Settings â†’ SDC Anywhere. */
export interface AnywhereStatus {
  enabled: boolean;
  running: boolean;
  connected: boolean;
  lastError: string | null;
  relay: string;
  daemonId: string | null;
  browserConnections: number;
  waitingApprovals: number;
  pairingRequests: number;
  keyStorage: { backend: 'os' | 'file'; protection: string; acceptedFileKey: boolean };
  settings: {
    notifyWhen: 'idle' | 'always' | 'never';
    idleMinutes: number;
    approvalTimeoutSec: number;
    onTimeout: 'pause' | 'deny';
    viewIdleLockMinutes: number;
    operateWindowMinutes: number;
    guestSessionMaxMinutes: number;
    maxScopedGrantMinutes: number;
    /** Where the relay sends a nudge or a sign-in link; empty = email off. */
    email: string;
    escalateEmailSec: number;
  };
}

export interface AnywhereDevice {
  id: string;
  name: string;
  userAgent: string;
  guest: boolean;
  createdAt: number;
  lastSeen: number | null;
  revokedAt: number | null;
  expiresAt: number | null;
}

export interface AnywherePairRequest {
  deviceId: string;
  name: string;
  userAgent: string;
  guest: boolean;
  /** The six digits (`123 456`) the phone shows too. Confirm only if they match. */
  code: string;
  askedAt: number;
}

export interface KillSwitchEvent {
  type: 'KillSwitch';
  stopped: ActiveWork[];
  checkpoints: { sessionId: string; checkpointId: string }[];
}

export interface TrustScoredEvent {
  type: 'TrustScored';
  sessionId: string;
  turnId: string;
  score: number;
  level: TrustLevel;
  reasons: TrustReason[];
}

export interface CheckpointUpdatedEvent {
  type: 'CheckpointUpdated';
  sessionId: string;
  checkpoint: CheckpointRecord & { sessionId?: string };
}

export interface FileRestoredEvent {
  type: 'FileRestored';
  sessionId: string;
  checkpointId: string;
  path: string;
  outcome: 'restored' | 'removed';
  undoCheckpointId: string | null;
}

export interface IntentParsedEvent {
  type: 'IntentParsed';
  intentId: string;
  sessionId: string | null;
  text: string;
  detection: Detection;
  spec: TaskSpec;
  /** Why the offline reading was used instead of a model's, when it was. */
  note: string | null;
}

export interface IntentConfirmedEvent {
  type: 'IntentConfirmed';
  sessionId: string | null;
  intentId: string;
  corrections: number;
}

export interface VoiceTranscribedEvent {
  type: 'VoiceTranscribed';
  requestId: string;
  text: string;
  engine: string | null;
  local: boolean;
  detection?: Detection;
  error: string | null;
  /** 1.0: a live caption while the person is still speaking - shown, never recorded. */
  interim?: boolean;
}

export interface DeployUpdatedEvent {
  type: 'DeployUpdated';
  deployId: string;
  siteId: string;
  name: string;
  kind: string;
  state: DeployState;
  steps: DeployStep[];
  backup: DeployRecord['backup'];
  note: string;
  health: HealthReport['http'] | null;
  actor: string;
}

export interface HealthUpdatedEvent {
  type: 'HealthUpdated';
  siteId: string;
  name: string;
  report: HealthReport;
  score: number;
  level: OpsLevel;
  reasons: TrustReason[];
}

export interface HealthAlertEvent {
  type: 'HealthAlert';
  siteId: string;
  name: string;
  level: 'critical' | 'warning' | 'info';
  sentence: string;
}

export interface GuardianActionEvent {
  type: 'GuardianAction';
  siteId: string;
  action: 'rolled_back' | 'alerted' | 'fix_prepared';
  sentence: string;
  deployId: string | null;
}

export interface ApprovalRecordedEvent {
  type: 'ApprovalRecorded';
  approval: ApprovalRecord;
}

export interface XrayReadyEvent {
  type: 'XrayReady';
  hostId: string;
  map: XrayMap | null;
  document?: string;
  path?: string | null;
  error: string | null;
}

export interface ShadowDbUpdatedEvent {
  type: 'ShadowDbUpdated';
  runId: string;
  siteId: string;
  state: 'running' | 'done';
  command: string;
  result?: { passed: boolean; exitCode: number | null; error: string | null; output: string[]; schemaChanges: string[]; sentence: string };
}

export interface StagingUpdatedEvent {
  type: 'StagingUpdated';
  siteId: string;
  approvalId: string;
  state: 'copying' | 'ready' | 'failed';
  url?: string;
  page?: string;
  php?: boolean;
  error?: string;
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
  | ToolCallDraftingEvent
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
  | SessionBridgedEvent
  | PlanUpdatedEvent
  | ResearchSourcesEvent
  | TurnSteeredEvent
  | ContextUpdatedEvent
  | QuestionAskedEvent
  | QuestionAnsweredEvent
  | VerifyUpdatedEvent
  | ModelsUpdatedEvent
  | CostUpdatedEvent
  | PolicyViolationEvent
  | BudgetStopEvent
  | KillSwitchEvent
  | RemoteActivityEvent
  | TrustScoredEvent
  | CheckpointUpdatedEvent
  | FileRestoredEvent
  | IntentParsedEvent
  | IntentConfirmedEvent
  | VoiceTranscribedEvent
  | DeployUpdatedEvent
  | HealthUpdatedEvent
  | HealthAlertEvent
  | GuardianActionEvent
  | ApprovalRecordedEvent
  | XrayReadyEvent
  | ShadowDbUpdatedEvent
  | StagingUpdatedEvent;

/** The `type` literals, in catalogue order â€” used by tests and by the reducer's exhaustiveness. */
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
  'ToolCallDrafting',
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
  'PlanUpdated',
  'ResearchSources',
  'TurnSteered',
  'ContextUpdated',
  'QuestionAsked',
  'QuestionAnswered',
  'VerifyUpdated',
  'ModelsUpdated',
  'CostUpdated',
  'PolicyViolation',
  'BudgetStop',
  'KillSwitch',
  'RemoteActivity',
  'TrustScored',
  'CheckpointUpdated',
  'FileRestored',
  'IntentParsed',
  'IntentConfirmed',
  'VoiceTranscribed',
  'DeployUpdated',
  'HealthUpdated',
  'HealthAlert',
  'GuardianAction',
  'ApprovalRecorded',
  'XrayReady',
  'ShadowDbUpdated',
  'StagingUpdated',
] as const satisfies readonly SdcpEvent['type'][];

/** Every event as a `Record` keyed by `type`, handy for a switch's exhaustiveness check. */
export type SdcpEventByType = {
  [K in SdcpEvent['type']]: Extract<SdcpEvent, { type: K }>;
};

/**
 * Method contracts â€” the `methods` block of the schema, in TypeScript.
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
      /** A known host, when `target` is empty (0.15.2): the daemon takes the address from its own row. */
      hostId?: string;
      label?: string;
      /**
       * The password for a host that asks for one, when the user chooses to give it.
       *
       * Since 0.8.1 it signs in once and that connection is kept open (`ssh::session::sign_in`); on a
       * machine without an `ssh` that can hold one, it copies SDC's public key instead
       * (`auth::remote::install_key`). Either way it is stored nowhere.
       */
      password?: string;
      /** The verification code a two-factor host asks for (0.8.1), spent with the password, kept nowhere. */
      code?: string;
      /**
       * Keep the password in the OS keychain once the host accepts it (0.14.4), so a dropped connection
       * asks only for the code. With a code and no password, a remembered password is used.
       */
      remember?: boolean;
      /**
       * "Stay signed in" (0.16.0): keep the password and the authenticator's key in the OS keychain, so a
       * dropped connection - or a restarted SDC - signs in again by itself. `false` forgets the key.
       */
      staySignedIn?: boolean;
      /** The authenticator's setup key (base32). Empty: the daemon reads `~/.google_authenticator` on the host. */
      totpSecret?: string;
    };
    /** `reused` is true when that `user@host` was already in the list - the row is returned as-is. */
    result: { hostId: string; reused: boolean };
  };
  /**
   * The answer to the trust question `host.add` may have asked (0.7.13).
   *
   * `fingerprint` is the string the dialog showed (`SHA256:â€¦`), and it is re-checked against the
   * machine before anything is written: a key that changed between the question and the answer is
   * refused rather than pinned. `password` is only spent **after** the pin, which is what makes the
   * one-time key install safe on a host whose identity was never checked.
   */
  'host.trust': {
    params: { hostId: string; fingerprint: string; password?: string; code?: string; remember?: boolean; staySignedIn?: boolean; totpSecret?: string };
    result: { trusted: boolean; hostId: string; fingerprint: string };
  };
  /**
   * One more probe of a host SDC already knows (0.11.3) - what `Reconnect` in the degraded banner calls.
   *
   * `status` is `connecting` while `ssh` dials and the probe's own verdict once it answers, which is the
   * same `HostStatus` the add/trust path pushes. `detail` carries the daemon's sentence either way, so the
   * host's row explains itself without a toast that the next launch would replay.
   */
  'host.probe': {
    params: { hostId: string };
    result: { hostId: string; status: string; detail?: string };
  };
  /**
   * The scan `host.add` does once, as a call (0.7.13).
   *
   * `matches` is `null` for a host SDC has no pin for (nothing to match against), `true` when the key is
   * the pinned one, and `false` when it is **not** - which is the case a `Re-pin` button confirms and the
   * case nothing in this daemon offers to "continue anyway" through.
   */
  /** Whether a host has a remembered password (0.14.4); `forget` deletes it. The password is never returned. */
  'host.password': { params: { hostId: string; forget?: boolean }; result: { saved: boolean; staysSignedIn?: boolean } };
  /** A dev server port on the chat's host as an address this machine can open (0.14.4). */
  /** A site, as a loopback address its frame-forbidding headers are taken off, for the preview (0.14.4). */
  'preview.open': { params: { url: string }; result: { url: string } };
  /** What the live site answers for a page (0.15.4): a 404 is a page that is not built and deployed yet. */
  'preview.status': { params: { url: string }; result: { status: number | null } };
  /** The project's own dev server as the preview (0.15.4): started on the chat's host, bound to loopback, reached through the connection. Call again until `ready`. */
  'preview.dev': {
    params: { sessionId: string; stop?: boolean };
    result: { state: 'starting' | 'ready' | 'failed' | 'stopped'; dir: string; port?: number; url?: string; log?: string };
  };
  'preview.forward': {
    params: { sessionId?: string; hostId?: string; port: number };
    result: { url: string; forwarded: boolean; localPort?: number };
  };
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
  /**
   * A project from nothing (0.9.0): the folder `<parent>/<name>` is created - locally or on the host -
   * and added as a project in the same call, so starting from scratch is one step. `name` must be a
   * plain folder name; separators and `..` are refused.
   */
  'project.scaffold': {
    params: { hostId?: string; parent: string; name: string };
    result: { projectId: string; hostId: string; root: string; name: string };
  };
  /**
   * Where a named thing lives on a machine (0.11.0) - the answer to *"give me my skilleddesk.com
   * project files"* when no project is bound yet. For a domain it reads the web server's own answer
   * first (an nginx `root` / Apache `DocumentRoot` whose server name matches), then the conventional
   * homes (`/var/www/<q>`, `/srv/<q>`, `~/<q>`, â€¦); locally it looks through the usual code folders.
   * Candidates are ordered best-first and every one is a real directory on that machine.
   */
  'project.locate': {
    params: { hostId?: string; query: string };
    result: { candidates: { root: string; source: string }[] };
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
      /**
       * Agent mode (v4). An API or Ollama model runs inside the daemon's own agent loop - it reads,
       * edits and runs commands in the chat's folder until the task is done. The three CLIs are agents
       * already, so for them the flag changes nothing.
       */
      agent?: boolean;
      /** Which actions wait for the person: `ask` (every change), `pro` (commands), `auto` (only dangerous ones). */
      autonomy?: 'ask' | 'pro' | 'auto';
      /** Model calls one agent turn may make before it pauses (default 60 since 0.13). */
      maxSteps?: number;
      /** `/compact` (0.13): the chat's model summarises the conversation; later turns start from the summary. */
      compact?: boolean;
      /** `/research` (0.16.1): an agent turn with the research brief, web tools, limits and numbered sources. */
      research?: boolean;
      /** How hard the model thinks (0.14): Claude Code `--effort`, Codex `model_reasoning_effort`, OpenAI `reasoning_effort`. */
      effort?: 'low' | 'medium' | 'high' | 'max';
      /** Images attached to the turn (0.13): base64, with their media type. */
      images?: { name: string; mediaType: string; data?: string; path?: string }[];
      /** A confirmed Intent Contract: the Prompt Compiler writes the engine's prompt from it (0.12). */
      intentId?: string;
      /** Read the message with SDC's brief - its language, every request in it, the answer's language (default true, 0.11.8). */
      understand?: boolean;
    };
    result: { turnId: string };
  };
  /**
   * The folder's checks, then a review of the change by another engine (v4). Answers at once; the run
   * streams as `VerifyUpdated`. `since` is the turn's first checkpoint (a shadow commit): the review
   * reads everything after it, new files included.
   */
  /** The tree's Rename (v4): inside the chat's folder only, a checkpoint first, never over an existing name. */
  'fs.rename': { params: { path: string; to: string; sessionId: string; hostId?: string }; result: { path: string; renamed: boolean } };
  /** The tree's Delete (v4): inside the chat's folder only, and a checkpoint first so Rewind restores it. */
  'fs.delete': { params: { path: string; sessionId: string; hostId?: string }; result: { path: string; deleted: boolean } };
  /** The tree's New folder (v4). */
  'fs.mkdir': { params: { path: string; sessionId: string; hostId?: string }; result: { path: string; created: boolean } };
  'verify.run': {
    params: {
      sessionId: string;
      turnId?: string;
      since?: string;
      task?: string;
      reviewer?: { engine: string; model: string; provider?: string };
      /** Review even when a check failed (by default the review waits for green checks). */
      reviewFailing?: boolean;
      hostId?: string;
      /** The conditions the reviewer judges one by one; read from `intentId` when not sent (0.12). */
      acceptance?: string[];
      intentId?: string;
      /** `security`: the role pipeline's SecReview. */
      focus?: 'security';
    };
    result: { verifyId: string };
  };
  /** `stopped`: whether an engine was found for the turn and told to stop, not only marked. */
  'engine.cancel': { params: { turnId: string }; result: { state: string; engine: string; stopped: boolean } };
  'engine.kill': { params: { turnId: string }; result: { state: string; engine: string; stopped: boolean } };
  /** Words for a running turn (0.12.5): an agent turn takes them between steps; `accepted: false` otherwise. */
  'engine.steer': { params: { turnId: string; text: string }; result: { accepted: boolean } };
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
    params: { query: string; glob?: string; root?: string; sessionId?: string; hostId?: string; limit?: number };
    /** `hits` are lines inside files; `files` (0.11.0) are files and folders whose *name* matches. */
    result: { hits: FsHit[]; files: FsFound[] };
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
      /** The Terminal's interactive shell (0.11.7): `ssh -tt` on a host (`tty: true`), the platform shell here. */
      shell?: boolean;
      cols?: number;
      rows?: number;
      cwd?: string;
      sessionId?: string;
      hostId?: string;
    };
    result: { ptyId: string; command: string; tty: boolean; hostId?: string | null };
  };
  'pty.write': { params: { ptyId: string; data: string }; result: Record<string, never> };
  'pty.resize': {
    params: { ptyId: string; cols: number; rows: number };
    result: Record<string, never>;
  };
  'pty.close': { params: { ptyId: string }; result: Record<string, never> };
  'pty.output': {
    /** With `since` (a byte offset), the raw output from there comes back as `data`, and `next` is where to ask from next (0.11.7). */
    params: { ptyId: string; since?: number };
    result: {
      ptyId: string;
      command: string;
      state: 'running' | 'exited' | 'gone';
      lines: string[];
      lineCount: number;
      ms: number;
      data?: string;
      next?: number;
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
      state: 'starting' | 'waiting_for_url' | 'waiting_for_code' | 'waiting_for_browser' | 'authenticated' | 'exited' | 'failed' | 'cancelled';
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
      /** The models in use (0.12.4): more than one, oldest first. */
      inUse: { modelId: string; providerId: string | null }[];
    };
  };
  /** Choose a model and put it in use - or, with `remove`, take it out of use without switching. */
  'models.select': {
    params: { modelId: string; providerId?: string; remove?: boolean };
    result: {
      modelId: string;
      providerId?: string | null;
      inUse: { modelId: string; providerId: string | null }[];
    };
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
    /** `id` is the card the key was saved on; `movedFrom` is set when that is not the one asked for. */
    result: { id: string; status: ProviderLifecycle; account?: string; movedFrom?: string };
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
    /** `delivered`: an agent was waiting on this question and has been given the answer. */
    result: { decision: string; delivered: boolean };
  };

  'console.attach': { params: { sessionId: string; url: string }; result: { attached: boolean } };
  'console.detach': { params: { sessionId: string }; result: { detached: boolean } };

  /* 0.12 ---------------------------------------------------------------------------------------- */
  'audit.list': { params: { sessionId?: string; turnId?: string; limit?: number }; result: { entries: AuditEntry[]; chain: LedgerCheck } };
  'audit.verify': { params: Record<string, never>; result: LedgerCheck };
  'policy.get': { params: { sessionId?: string; root?: string; hostId?: string }; result: { root: string | null; path: string | null; exists: boolean; policy: Policy; text: string; defaults: { protected: string[]; alwaysAsk: string[] } } };
  'policy.set': { params: { sessionId?: string; root?: string; hostId?: string; text?: string; policy?: Partial<Policy> }; result: { policy: Policy; path: string } };
  'kill.all': { params: Record<string, never>; result: { stopped: ActiveWork[]; processes: number; checkpoints: { sessionId: string; checkpointId: string }[] } };
  'kill.list': { params: Record<string, never>; result: { active: ActiveWork[] } };
  'anywhere.status': { params: Record<string, never>; result: AnywhereStatus };
  'anywhere.enable': { params: Record<string, never>; result: AnywhereStatus };
  'anywhere.disable': { params: Record<string, never>; result: AnywhereStatus };
  'anywhere.configure': { params: { relay?: string; acceptFileKey?: boolean; notifyWhen?: 'idle' | 'always' | 'never'; idleMinutes?: number; approvalTimeoutSec?: number; onTimeout?: 'pause' | 'deny'; viewIdleLockMinutes?: number; operateWindowMinutes?: number; guestSessionMaxMinutes?: number; maxScopedGrantMinutes?: number; email?: string; escalateEmailSec?: number }; result: AnywhereStatus };
  'anywhere.pair.begin': { params: { guest?: boolean }; result: { url: string; fingerprint: string; expiresAt: number; guest: boolean } };
  'anywhere.pair.requests': { params: Record<string, never>; result: { requests: AnywherePairRequest[] } };
  'anywhere.pair.confirm': { params: { deviceId: string; accept?: boolean }; result: { paired: boolean } };
  'anywhere.devices.list': { params: Record<string, never>; result: { devices: AnywhereDevice[] } };
  'anywhere.devices.revoke': { params: { deviceId: string }; result: { revoked: boolean } };
  'anywhere.reset': { params: Record<string, never>; result: { reset: boolean } };
  'cost.summary': { params: Record<string, never>; result: CostSummary };
  'cost.estimate': { params: { prompt: string; engine: string; model: string; provider?: string; sessionId?: string; agent?: boolean }; result: { estimate: CostEstimate; cheaper: CheaperModel | null; spent: CostSpent; budgets: CostBudgets } };
  'cost.budget.set': { params: { turn?: number | null; chat?: number | null; day?: number | null; month?: number | null; baseline?: string }; result: CostSummary };
  'trust.score': { params: { turnId: string }; result: { turnId: string; sessionId: string; score: number; level: TrustLevel; reasons: TrustReason[]; ts: string } | null };
  'checkpoint.label': { params: { checkpointId: string; label?: string }; result: { checkpoint: CheckpointRecord } };
  'checkpoint.files': { params: { checkpointId: string }; result: { checkpoint: CheckpointRecord; files: { status: string; path: string }[] } };
  'checkpoint.fileDiff': { params: { checkpointId: string; path: string }; result: { path: string; diff: string } };
  'checkpoint.restoreFile': { params: { checkpointId: string; path: string }; result: { path: string; outcome: 'restored' | 'removed'; undoCheckpointId: string } };
  'proof.export': { params: { sessionId: string; turnId?: string; lang?: string }; result: { jsonPath: string; htmlPath: string; html: string; pack: Record<string, unknown> } };
  'intent.detect': { params: { text: string }; result: Detection };
  'intent.parse': { params: { text: string; sessionId?: string; engine?: string; model?: string; provider?: string; hostId?: string }; result: { intentId: string; detection: Detection } };
  'intent.confirm': { params: { intentId: string; spec?: TaskSpec; glossary?: { term: string; meaning: string }[]; sessionId?: string }; result: { intentId: string; spec: TaskSpec; corrections: number; glossaryScope: string } };
  'intent.compile': { params: { intentId: string; engine?: string; sessionId?: string }; result: { engine: string; prompt: string } };
  'intent.cancel': { params: { intentId: string }; result: { cancelled: boolean } };
  'intent.stats': { params: Record<string, never>; result: { parsed: number; confirmed: number; corrected: number; cancelled: number } };
  'glossary.list': { params: { sessionId?: string; scope?: string }; result: { scope: string; terms: GlossaryTerm[] } };
  'glossary.set': { params: { sessionId?: string; scope?: string; term: string; meaning?: string }; result: { scope: string; terms: GlossaryTerm[] } };
  'voice.status': { params: Record<string, never>; result: { local: boolean; program: string | null; model: string | null; online: string[]; available: boolean; hint: string | null } };
  'voice.transcribe': { params: { audio: string; mime?: string; language?: string; sessionId?: string; interim?: boolean }; result: { requestId: string } };
  'site.list': { params: Record<string, never>; result: { sites: SiteRecord[] } };
  'site.detect': { params: { hostId?: string; root: string }; result: { config: Record<string, unknown> } };
  'site.save': { params: { siteId?: string; name: string; hostId?: string; root: string; url?: string; config?: Record<string, unknown> }; result: { siteId: string; site: SiteRecord } };
  'site.remove': { params: { siteId: string }; result: { removed: boolean } };
  'deploy.run': { params: { siteId: string; kind?: 'production' | 'staging' }; result: { deployId?: string; state: 'running' | 'awaiting_approval'; approvalId?: string } };
  'deploy.list': { params: { siteId?: string; limit?: number }; result: { deploys: DeployRecord[] } };
  'deploy.get': { params: { deployId: string }; result: { deploy: DeployRecord | null } };
  'deploy.preview': { params: { deployId: string }; result: { changes: { path: string; change: 'added' | 'modified' | 'deleted' }[]; archive: string } };
  'deploy.rollback': { params: { deployId: string }; result: { state: 'running' } };
  'deploy.restoreDb': { params: { deployId: string; confirm: 'RESTORE' }; result: { restored: boolean; output: string[] } };
  'health.check': { params: { siteId: string }; result: { queued: boolean } };
  'health.history': { params: { siteId: string; limit?: number }; result: { reports: HealthReport[] } };
  'guardian.set': { params: { siteId: string; enabled: boolean; autoRollback: boolean }; result: { guardian: { enabled: boolean; autoRollback: boolean } } };
  'approval.list': { params: { subject?: string }; result: { approvals: ApprovalRecord[] } };
  'approval.request': { params: { subject: string; kind?: string; note?: string; token?: string }; result: { approval: ApprovalRecord } };
  'approval.decide': { params: { approvalId: string; decision: 'approved' | 'declined'; note?: string }; result: { approval: ApprovalRecord } };
  'approval.poll': { params: { approvalId: string }; result: { approval: ApprovalRecord; answered?: boolean } };
  'xray.scan': { params: { hostId: string }; result: { queued: boolean } };
  'xray.get': { params: { hostId: string }; result: { map: XrayMap | null } };
  'shadowdb.run': { params: { siteId: string; command: string }; result: { runId: string } };
  'staging.create': { params: { siteId: string; summary?: string; changes?: string[]; before?: string; after?: string; lang?: string }; result: { approvalId: string } };
  'staging.stop': { params: { siteId: string; remove?: boolean }; result: { stopped: boolean } };
  'playbook.list': { params: Record<string, never>; result: { playbooks: Playbook[] } };
  'playbook.save': { params: { playbook: Partial<Playbook> }; result: { playbooks: Playbook[] } };
  'playbook.remove': { params: { playbookId: string }; result: { playbooks: Playbook[] } };
  'playbook.run': { params: { playbookId: string; siteIds: string[] }; result: { deploys: { siteId: string; deployId: string }[]; prompts: string[] } };
  'team.get': { params: Record<string, never>; result: TeamState };
  'team.set': { params: { members?: TeamMember[]; current?: string }; result: TeamState };
  'settings.get': { params: { key: string }; result: { key: string; value: string | null } };
  'settings.set': { params: { key: string; value: string | boolean }; result: { key: string; value: string } };
  'update.check': { params: { channel?: 'stable' | 'beta' }; result: UpdateInfo };
  'tool.list': { params: Record<string, never>; result: { tools: ToolRow[]; root: string } };
  'tool.install': { params: { id: string; hostId?: string }; result: ToolJob };
  'tool.status': { params: { jobId: string }; result: ToolJob };
  'ollama.start': { params: Record<string, never>; result: { running: boolean } };
  'ollama.pull': { params: { model: string }; result: ToolJob };
  'host.port.free': { params: { port: number }; result: { stopped: number } };
  'provider.warm': { params: { model: string; provider?: string }; result: { warming: boolean } };
  'local.discover': { params: Record<string, never>; result: { servers: LocalServer[] } };
  'local.add': { params: { url: string; label?: string; key?: string }; result: { id: string; label: string; base: string; models: LocalModel[] } };
  'local.remove': { params: { id: string }; result: { removed: string } };
  'crash.list': { params: Record<string, never>; result: { reports: CrashReport[] } };
  'crash.clear': { params: Record<string, never>; result: { cleared: number } };
  'cli.selfcheck': { params: Record<string, never>; result: { clis: CliSelfCheck[] } };
  'status.share': { params: { enabled: boolean; port?: number }; result: { enabled: boolean; port?: number; url?: string; error?: string } };
  'timeline.branches': { params: { sessionId: string }; result: { branches: TimelineBranch[] } };
  'timeline.switch': { params: { sessionId: string; frameId: number }; result: { switched: boolean; turn?: number } };
  /** The person's answer to an agent's `ask_user` (0.13). */
  'question.answer': { params: { questionId: string; answer: string; turnId?: string; sessionId?: string }; result: { answered: boolean } };
  /** The project's `.sdc/memory.md` (scope `project`, on the chat's machine) or SDC's global memory (0.13). */
  'memory.get': { params: { sessionId?: string; scope?: 'project' | 'global' }; result: { text: string; path: string } };
  'memory.set': { params: { sessionId?: string; scope?: 'project' | 'global'; text: string }; result: { saved: boolean; path: string } };
  /** `/remember <fact>`: one line appended. */
  'memory.add': { params: { sessionId?: string; scope?: 'project' | 'global'; text: string }; result: { saved: boolean; path: string; text: string } };
  /** The composer's `/` list: SDC's commands, then the project's `.sdc/commands` and `.claude/commands` files. */
  'commands.list': { params: { sessionId?: string }; result: { commands: { name: string; description: string; source: string; body?: string }[] } };
  /** The composer's `@` list: files under the chat's folder whose name contains the query. */
  'files.find': { params: { sessionId: string; query: string; limit?: number }; result: { files: { path: string; dir: boolean }[] } };
  /** Processes an agent left running (a dev server), per chat. */
  'process.list': { params: { sessionId?: string }; result: { processes: { processId: string; sessionId: string; command: string; place: string; seconds: number; running: boolean }[] } };
  'process.stop': { params: { processId: string }; result: { stopped: boolean } };
  /** The context meter before a turn: what the next turn of the chat would send to this model. */
  'context.get': { params: { sessionId: string; engine?: string; model?: string; provider?: string }; result: { usedTokens: number; windowTokens: number; percent: number; compacted: boolean; resumed: boolean } };
  /** Settings â†’ Research (0.16.1): the search service, which keyed services have a key (masked), limits, the final-answer model, and Ollama. */
  'research.status': { params: Record<string, never>; result: ResearchStatus };
  /** A search service's key into the OS keychain; an empty key removes it. */
  'research.key.set': { params: { provider: 'tavily' | 'brave' | 'serper'; key: string }; result: ResearchStatus };
  /** What a `/research` will do - model, place, search service, limits, estimated cost - before it starts. */
  'research.plan': { params: { engine: string; provider?: string; model: string; prompt: string }; result: ResearchPlan };
  /** Settings â†’ Erase all SDC data: keys now, the data folder on the daemon's next start. `confirm` must be `ERASE`. */
  'app.erase': { params: { confirm: 'ERASE' }; result: { erasing: boolean } };
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


/** One tool SDC can install itself (0.21, `tool.list`). */
export interface ToolRow {
  id: string;
  label: string;
  installed: boolean;
  version: string | null;
  /** SDC put it there (in its own tools folder), rather than the person. */
  managed: boolean;
  /** Roughly what the download weighs. */
  size: string;
  /** The install running for it now, if any. */
  running: ToolJob | null;
}

/** An install (or an Ollama model download) running on the daemon (0.21). */
export interface ToolJob {
  id: string;
  tool: string;
  state: 'running' | 'done' | 'failed';
  step: string;
  done: number;
  total: number | null;
  log: string[];
  error: string | null;
}

/** A model a local server lists, with the context it runs with (0.22). */
export interface LocalModel {
  id: string;
  ctx: number;
}

/** An OpenAI-compatible model server found on this computer (0.22, `local.discover`). */
export interface LocalServer {
  id: string;
  label: string;
  base: string;
  models: LocalModel[];
  /** Already connected as a provider. */
  connected: boolean;
  /** Why it could not be listed (a server started with a key). */
  note: string | null;
}
