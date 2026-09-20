import type { PermissionDecision, PermissionRisk, TierName } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';
import { useOverlayStore } from './overlays';
import { usePrefsStore } from './prefs';
import { dispatch, useAppStore } from './store';

/**
 * The intents - the UI's verbs (master spec section 3.3: "the UI never mutates state directly; it
 * dispatches intents, the daemon appends events, the UI reducer projects events → view state").
 *
 * Every function here does the same three things, in the same order:
 *
 *   1. call the daemon (`sdcpCall`), whose answer arrives as a *notification* stream;
 *   2. let the transport put those notifications in the event log - nothing here touches state;
 *   3. report the outcome, either by letting the daemon's own `Toast` event through or, when the
 *      call *fails*, by appending one local `Toast` event that says so in plain words.
 *
 * Point 3 is the only place this file writes to the log, and it writes exactly one kind of event.
 * That is deliberate: an optimistic patch here would be a second source of truth, and a silent
 * failure would be the lie principle P4 forbids. A failed call is visible.
 *
 * The signatures return promises so a caller can drive a spinner from them - which is how the
 * Provider Hub's `Test` button gets its 1.2 seconds of `Testing…`.
 */

/** Appends a local toast. The only event the UI is allowed to originate. */
export function toast(message: string, action?: string, holdMs?: number): void {
  dispatch(
    action === undefined ? { type: 'Toast', message } : { type: 'Toast', message, action, holdMs },
  );
}

/** `Failed · <reason>` for anything the daemon rejected, honouring the error's own wording. */
function reportFailure(error: unknown, fallback: string): void {
  toast(isSdcpError(error) ? error.message : fallback);
}

/* ------------------------------------------------------------------------------------------------
 * The daemon handshake
 * ---------------------------------------------------------------------------------------------- */

/**
 * Called once, when the window mounts (spec sections 3.1, 5.4).
 *
 * It asks `host.status` for two reasons, and the first one is the important one on a fresh install:
 *
 *   1. **it starts the daemon.** The Tauri bridge connects lazily, and a connection that is refused
 *      makes it spawn `sdcd` (next to the app binary) and wait for the port. Without this call an
 *      installed app would sit there looking fine until the user's first action;
 *   2. **it folds the real machine into the store.** `host.status` pushes a `HostStatus` event, so
 *      the host list, the topbar dot and the status bar describe *this* machine - engines, keychain
 *      backend, event count - rather than the seed.
 *
 * A failure is reported once, and only once: a daemon that is not there is a fact the user should see,
 * not a toast per action (principle P4). It is deliberately not retried here - every intent retries
 * on its own, so the next click is the retry.
 */
export async function connectDaemon(): Promise<boolean> {
  try {
    await sdcpCall('host.status', {});

    return true;
  } catch (error) {
    toast(isSdcpError(error) ? error.message : strings.daemon.offline);

    return false;
  }
}

/* ------------------------------------------------------------------------------------------------
 * Sessions and hosts
 * ---------------------------------------------------------------------------------------------- */

/** Create an empty session on a host and focus its prompt (spec sections 7.3, 9.5). */
export async function newChatOnHost(hostId: string): Promise<string | null> {
  try {
    const { sessionId } = await sdcpCall('session.open', {
      hostId,
      title: strings.sidebar.sessions.newChat.title,
      prompt: strings.sidebar.sessions.newChat.prompt,
    });

    toast(strings.sidebar.newChatOn(hostId));

    return sessionId;
  } catch (error) {
    reportFailure(error, 'Could not start that chat');
    return null;
  }
}

export async function renameSession(sessionId: string, title: string): Promise<void> {
  try {
    await sdcpCall('session.update', { sessionId, title });
  } catch (error) {
    reportFailure(error, 'Rename failed');
  }
}

export async function closeSession(sessionId: string): Promise<void> {
  try {
    await sdcpCall('session.close', { sessionId });
    toast(strings.sidebar.deleted);
  } catch (error) {
    reportFailure(error, 'Delete failed');
  }
}

/** Spec section 9.12. The daemon answers immediately; the host turns `connected` 1.4s later. */
export async function addHost(input: {
  type: 'local' | 'ssh';
  target?: string;
  label?: string;
}): Promise<string | null> {
  if (input.type === 'local') {
    toast(strings.addHost.localAlready);
    return 'local';
  }

  if (!input.target || input.target.trim() === '') {
    toast(strings.addHost.needTarget);
    return null;
  }

  try {
    const { hostId } = await sdcpCall('host.add', input);

    return hostId;
  } catch (error) {
    reportFailure(error, 'Could not connect to that host');
    return null;
  }
}

/* ------------------------------------------------------------------------------------------------
 * Providers (spec section 9.10)
 * ---------------------------------------------------------------------------------------------- */

export interface ProviderTestResult {
  ok: boolean;
  models: number;
  detail: string;
  /**
   * Whether the **provider** was actually contacted (`false` for a key that was only shape-checked,
   * which is what this build can do for an `https://` endpoint). The card shows `detail`, so the
   * sentence the user reads is the truth either way.
   */
  verified?: boolean;
  /** `null` when the check passed and the daemon sent an explicit empty, a sentence otherwise. */
  error?: string | null;
}

/** Flow 1's `Test`: 1.2s of spinner, then `OK · key valid · 12 models available`. */
export async function testProvider(id: string, key: string): Promise<ProviderTestResult | null> {
  try {
    return await sdcpCall('provider.test', key === '' ? { id } : { id, key });
  } catch (error) {
    return {
      ok: false,
      models: 0,
      detail: '',
      error: isSdcpError(error) ? error.message : 'failed',
    };
  }
}

/** Flow 1's `Save`: the key goes to the keychain; the UI only ever sees the masked label. */
export async function connectApiKey(id: string, key: string, label?: string): Promise<boolean> {
  if (key.trim() === '') {
    toast(strings.hub.keyToast);
    return false;
  }

  try {
    await sdcpCall('provider.save', {
      id,
      kind: 'api-key',
      key,
      ...(label === undefined || label === '' ? {} : { label }),
    });

    return true;
  } catch (error) {
    reportFailure(error, 'Could not save that key');
    return false;
  }
}

/** Flow 2: open the browser, wait, receive the token. */
export async function authorizeSubscription(id: string): Promise<boolean> {
  try {
    const { url, state } = await sdcpCall('provider.oauth.open', { id });

    window.open(url, '_blank', 'noopener,noreferrer');

    const { ok } = await sdcpCall('provider.oauth.callback', { id, state });

    return ok;
  } catch (error) {
    reportFailure(error, 'Authorization failed');
    return false;
  }
}

/** Flow 3: the Local (Ollama) flow's two doctor rows. */
export async function connectLocal(): Promise<{ daemon: boolean; models: string[] } | null> {
  try {
    return await sdcpCall('provider.local.doctor', {});
  } catch (error) {
    reportFailure(error, 'Ollama is not reachable');
    return null;
  }
}

/** Flow 4: any OpenAI- or Anthropic-compatible endpoint. */
export async function saveCustomEndpoint(input: {
  url: string;
  key: string;
  protocol: string;
}): Promise<boolean> {
  if (input.url.trim() === '') {
    toast(strings.addHost.needTarget);
    return false;
  }

  try {
    await sdcpCall('provider.save', {
      id: 'custom',
      kind: 'custom',
      key: input.key,
      url: input.url,
      protocol: input.protocol,
    });
    toast(strings.hub.endpointToast);

    return true;
  } catch (error) {
    reportFailure(error, 'Could not save that endpoint');
    return false;
  }
}

/** Flow 5: enable or disable one model. The daemon answers with the whole registry. */
export async function toggleRegistryModel(id: string, enabled: boolean): Promise<void> {
  try {
    await sdcpCall('provider.registry.set', { id, enabled });
  } catch (error) {
    reportFailure(error, 'Could not change that model');
  }
}

/** Flow 6: the ten environment checks (spec section 9.10). */
export async function runDoctor(hostId = 'local'): Promise<void> {
  try {
    const { checks } = await sdcpCall('host.doctor', { hostId });

    /* The checks are a read's *result*, so they land as one state patch rather than as events. */
    const { useAppStore } = await import('./store');
    const { withDoctorRun } = await import('./reducer');

    useAppStore.setState((state) =>
      withDoctorRun(
        state,
        hostId,
        checks.map((check) => ({
          id: check.id,
          label: check.label,
          state: check.state,
          detail: check.detail,
          fix: check.fix ?? null,
        })),
      ),
    );
  } catch (error) {
    reportFailure(error, 'Doctor run failed');
  }
}

/* ------------------------------------------------------------------------------------------------
 * Permission (spec section 9.13)
 * ---------------------------------------------------------------------------------------------- */

export interface PermissionSeed {
  sessionId: string;
  turnId: string;
  action: string;
  target: string;
  risk: PermissionRisk;
}

/** Ask the daemon to raise the approval dialog. The dialog's content comes back as an event. */
export async function requestPermission(seed: PermissionSeed): Promise<string | null> {
  try {
    const { permissionId } = await sdcpCall('permission.request', seed);

    return permissionId;
  } catch (error) {
    reportFailure(error, 'Could not ask for permission');
    return null;
  }
}

/**
 * The UI's entrance to the dialog: ask, then show. The two steps are not the same thing - the
 * *content* is an event, the *visibility* is a UI preference - so this helper exists to keep a
 * caller from remembering both.
 *
 * `seed` is partial because the three entrances know different amounts: the error card has a title
 * and an explanation, the console a file and a line, and a bare trigger only knows the session.
 */
export async function askPermission(seed: Partial<PermissionSeed> = {}): Promise<void> {
  const permissionId = await requestPermission({
    sessionId: seed.sessionId ?? usePrefsStore.getState().activeTab ?? 's1',
    turnId: seed.turnId ?? useAppStore.getState().activeTurnId ?? '',
    action: seed.action ?? 'delete',
    target: seed.target ?? strings.permission.target,
    risk: seed.risk ?? 'MUTATING',
  });

  if (permissionId !== null) {
    useOverlayStore.getState().openPermission();
  }
}

/** The four decisions of spec section 9.13; the daemon answers with `PermissionResolved`. */
export async function resolvePermission(
  permissionId: string,
  decision: PermissionDecision,
  scope?: string,
): Promise<void> {
  try {
    await sdcpCall('permission.resolve', {
      permissionId,
      decision,
      ...(scope === undefined ? {} : { scope }),
    });
  } catch (error) {
    reportFailure(error, 'Could not record that decision');
  }
}

/* ------------------------------------------------------------------------------------------------
 * Turns, the Session Bridge, the Time Machine, Duel and the console bridge
 * (spec sections 9.1's session keys, 14, 15.4, 16.5, 16.6)
 * ---------------------------------------------------------------------------------------------- */

export interface TurnSeed {
  sessionId: string;
  prompt: string;
  engine: string;
  model: string;
  tier: TierName;
}

/** `Enter` in the prompt area: the daemon appends `TurnStarted` and starts streaming. */
export async function startTurn(seed: TurnSeed): Promise<string | null> {
  if (seed.prompt.trim() === '') {
    toast(strings.prompt.empty);
    return null;
  }

  try {
    const { turnId } = await sdcpCall('engine.start', seed);

    return turnId;
  } catch (error) {
    reportFailure(error, 'The engine did not start');
    return null;
  }
}

/** `Esc`. */
export async function interruptTurn(turnId: string): Promise<void> {
  try {
    await sdcpCall('engine.cancel', { turnId });
    toast(strings.prompt.interrupt);
  } catch (error) {
    reportFailure(error, 'Interrupt failed');
  }
}

/** `Ctrl+Shift+Esc` - the difference between asking and insisting. */
export async function forceKillTurn(turnId: string): Promise<void> {
  try {
    await sdcpCall('engine.kill', { turnId });
    toast(strings.prompt.killed);
  } catch (error) {
    reportFailure(error, 'Kill failed');
  }
}

/** Spec section 16.5: switch engines mid-turn and replay the context into the new one. */
export async function bridgeEngine(
  turnId: string,
  engine: string,
  model: string,
  reason?: string,
): Promise<void> {
  try {
    await sdcpCall('engine.switch', { turnId, engine, model, ...(reason === undefined ? {} : { reason }) });
  } catch (error) {
    reportFailure(error, 'Engine switch failed');
  }
}

/** Spec section 14: rewind to a checkpoint. The daemon emits `RewindApplied` and a 10s toast. */
export async function rewindTo(sessionId: string, turnId: string): Promise<void> {
  try {
    await sdcpCall('rewind.apply', { sessionId, turnId });
  } catch (error) {
    reportFailure(error, 'Rewind failed');
  }
}

/** `Ctrl+Shift+Z`. */
export async function redoRewind(sessionId: string): Promise<void> {
  try {
    await sdcpCall('rewind.redo', { sessionId });
  } catch (error) {
    reportFailure(error, 'Redo failed');
  }
}

/** Spec section 16.6: the same prompt, two engines. */
export async function startDuel(
  sessionId: string,
  prompt: string,
  engines: string[],
): Promise<string | null> {
  try {
    const { duelId } = await sdcpCall('duel.start', { sessionId, prompt, engines });

    return duelId;
  } catch (error) {
    reportFailure(error, 'Duel could not start');
    return null;
  }
}

/** `Keep` archives the other run; `Keep neither` archives both (spec section 16.6). */
export async function keepDuel(duelId: string, keep: string | null): Promise<void> {
  try {
    if (keep === null) {
      await sdcpCall('duel.discard', { duelId });
      return;
    }

    await sdcpCall('duel.keep', { duelId, keep });
  } catch (error) {
    reportFailure(error, 'Could not archive that run');
  }
}

/** Spec section 15.4: attach the preview's console so its errors can become turns. */
export async function attachConsole(sessionId: string, url: string): Promise<void> {
  try {
    await sdcpCall('console.attach', { sessionId, url });
  } catch (error) {
    reportFailure(error, 'Could not attach the console');
  }
}

/**
 * Spec sections 14.9 and 15.4: hand a failure to the agent.
 *
 * `Fix this` and `Fix with agent` are the same action with two entrances - an error card in the
 * turn stream and a console row in the right panel - so they share one intent. The new turn is
 * seeded with the failure's context (file, line, message) rather than with the word "fix", because
 * an agent that has to guess which failure you meant will guess wrong.
 */
export async function fixWithAgent(input: {
  sessionId: string;
  engine: string;
  model: string;
  tier: TierName;
  title: string;
  explanation: string;
  source?: string;
  file?: string;
  line?: number;
}): Promise<void> {
  const location =
    input.file === undefined
      ? (input.source ?? '')
      : `${input.file}${input.line === undefined ? '' : `:${input.line}`}`;

  const prompt = [input.title, location, input.explanation].filter((part) => part !== '').join('\n');

  await startTurn({
    sessionId: input.sessionId,
    prompt,
    engine: input.engine,
    model: input.model,
    tier: input.tier,
  });
}

