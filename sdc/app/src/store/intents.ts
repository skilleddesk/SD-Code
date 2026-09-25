import type { FsEntry, PermissionDecision, PermissionRisk, TierName } from '../../../protocol/types';
import { nameOf, pickFolder } from '../lib/picker';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { baseName, inFolder } from '../lib/paths';
import { strings } from '../strings';
import { useDaemonStore } from './daemon';
import { useFilesStore } from './files';
import { useLayoutStore } from './layout';
import { groupCatalog, useModelStore, engineForProvider, tierName, tierFromName } from './model';
import { useOverlayStore } from './overlays';
import { usePrefsStore } from './prefs';
import { withProjects, withProviders, withWorkspace } from './reducer';
import { useRightPanelStore } from './rightPanel';
import { selectActiveSession, dispatch, useAppStore } from './store';
import { findSession } from './sessions';
import { useTerminalStore } from './terminal';
import type { AppState, HostView, TurnView } from './types';

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

    /*
     * And the provider list, which 0.6.0 asked for and 0.6.1 finally keeps.
     *
     * `provider.list` pushes nothing - the daemon answers with the cards and appends no event - so
     * calling it was only half the job: the Hub, the topbar's plug and the status bar all read the
     * event log's fold, and that fold stayed empty, which is why the connect screen drew
     * `0 connected · None yet` next to a daemon that knew about eleven providers. The answer is folded
     * here, the way `session.list`'s is, because a list is a read's result and not a stream of events.
     *
     * A failure is not fatal: the window keeps working with an empty provider list, and the toast has
     * already said the daemon is not answering.
     */
    const { providers } = await sdcpCall('provider.list', {});

    useAppStore.setState((state) => withProviders(state, providers));

    /*
     * And the list the window used to lose on every launch.
     *
     * `host.status` records *this* machine; `session.list` is what the daemon already had - every
     * host that was ever added, and every chat under it. Without this call the sidebar showed one
     * host after a restart and the added ones looked as though adding them had failed.
     */
    await loadWorkspace();

    /*
     * And the model catalogue, which is what the prompt toolbar's dropdown is built from.
     *
     * 0.7.0 made that list the daemon's own: before this call the dropdown had a hardcoded list of
     * invented names, which is what "the model thing above the chat box is dummy" was about. A failure
     * is not fatal - the dropdown says it has nothing verified yet and offers to connect one.
     */
    await refreshCatalog();

    return true;
  } catch (error) {
    toast(isSdcpError(error) ? error.message : strings.daemon.offline);

    return false;
  }
}

/* ------------------------------------------------------------------------------------------------
 * Signing a CLI in, and choosing a model (spec section 9.10)
 *
 * These two are the flows a user actually needs to get *working*: a link they approve in a browser and
 * a code they paste back, or an API key plus the model to use it with. Neither writes state here: the
 * daemon's `ProviderStatus` event moves the card, and the model choice is a setting the daemon keeps.
 * ---------------------------------------------------------------------------------------------- */

/** What `cli.login.status` reports, as the UI needs it. */
export interface CliLoginView {
  loginId: string;
  providerId: string;
  providerLabel: string;
  url: string | null;
  state: 'starting' | 'waiting_for_url' | 'waiting_for_code' | 'authenticated' | 'exited' | 'failed' | 'cancelled';
  note: string | null;
  lines: string[];
  authenticated: boolean;
  ms: number;
}

/**
 * One row of `cli.recipes`: how a subscription provider's CLI is signed in, and whether **this** machine
 * has the program at all (0.7.8).
 *
 * The `installed` flag is the whole point of reading this before a sign-in starts: `claude`, `codex` and
 * `gemini` are separate programs, and "Connect" on a provider whose program is missing used to *start* the
 * login and then fail with the reason - so the sentence a person needed arrived as the explanation of a
 * failure instead of as a step they could take first.
 */
export interface CliRecipeView {
  providerId: string;
  label: string;
  program: string;
  note: string;
  installed: boolean;
}

/**
 * The recipe for one provider, or `null` when it has no CLI sign-in (an API-key provider).
 *
 * The daemon's `cli.recipes` answers all of them with `installed` checked by the same `doctor::has` the
 * environment doctor uses, so the two cannot disagree about whether a program is there.
 */
export async function loadCliRecipe(providerId: string): Promise<CliRecipeView | null> {
  try {
    const { recipes } = await sdcpCall('cli.recipes', {});

    return recipes.find((recipe) => recipe.providerId === providerId) ?? null;
  } catch {
    /* A recipe that cannot be read is not worth a toast: the dialog still has its Sign in button, and the
       daemon's own sentence arrives if the program really is missing. */
    return null;
  }
}

/** The model catalogue, as `models.list` answers it. */
export interface ModelsView {
  models: {
    id: string;
    name: string;
    providerId: string;
    providerLabel: string;
    tier: TierName;
    ctx: number;
    cost: string;
    source: 'live' | 'cache' | 'bundled';
    fetchedAt?: string | null;
  }[];
  snapshot: string;
  refreshed: boolean;
  notes: string[];
  selected: { modelId: string | null; providerId: string | null };
}

/**
 * `models.list` without a refresh: what the daemon's catalogue holds right now.
 *
 * It is the same call the Provider Hub makes, folded into the model store instead of a local
 * component state, because the dropdown above the prompt box is where the answer has to be *visible*:
 * a signed-in plan's models, and nothing that cannot run. No provider is contacted by this call - a
 * `refresh` is what asks, and only the Hub has a button for that.
 */
export async function refreshCatalog(): Promise<void> {
  try {
    const { models } = (await sdcpCall('models.list', {})) as ModelsView;

    useModelStore.getState().setCatalog(models.map((row) => ({ ...row, tier: tierFromName(row.tier) })));
  } catch {
    /* A window with no daemon keeps its fallback list; the boot toast has already explained why. */
  }
}

/** Starts the CLI's own sign-in. The URL arrives on the first poll, a moment later. */
export async function startCliLogin(providerId: string): Promise<CliLoginView | null> {
  try {
    const { loginId } = await sdcpCall('cli.login', { providerId });

    return await pollCliLogin(loginId);
  } catch (error) {
    reportFailure(error, strings.connect.loginFailed);
    return null;
  }
}

/** One poll of an in-flight login: the URL, the CLI's own tail, and where it has got to. */
export async function pollCliLogin(loginId: string): Promise<CliLoginView | null> {
  try {
    return (await sdcpCall('cli.login.status', { loginId })) as CliLoginView;
  } catch (error) {
    reportFailure(error, strings.connect.loginFailed);
    return null;
  }
}

/** Hands the pasted code to the CLI, which is the only thing that can use it. */
export async function submitCliLoginCode(loginId: string, code: string): Promise<CliLoginView | null> {
  try {
    await sdcpCall('cli.login.code', { loginId, code });

    return await pollCliLogin(loginId);
  } catch (error) {
    reportFailure(error, strings.connect.codeFailed);
    return null;
  }
}

export async function cancelCliLogin(loginId: string): Promise<void> {
  try {
    await sdcpCall('cli.login.cancel', { loginId });
  } catch (error) {
    reportFailure(error, strings.connect.loginFailed);
  }
}

/**
 * The model list. `refresh` asks each provider's own endpoint; a row's `source` says whether it is
 * live, cached or the bundle's, and a failed refresh explains itself in `notes` instead of throwing
 * the list away.
 */
export async function loadModels(providerId: string | null, refresh: boolean): Promise<ModelsView | null> {
  try {
    return (await sdcpCall('models.list', {
      ...(providerId === null ? {} : { providerId }),
      refresh,
    })) as ModelsView;
  } catch (error) {
    reportFailure(error, strings.connect.modelsFailed);
    return null;
  }
}

/**
 * Records the chosen model so the prompt area and the status bar use it.
 *
 * **Two places, and the second one was missing.** `models.select` tells the daemon which model the
 * setting is, and until 0.7.2 that was the whole function - the window's own model store was never
 * touched. The symptom was reported as *"connect hoy claude but chat e kisu likhle kaj hoy nah"*: press
 * `Use` on a row in the Connect dialog, the row says `In use`, and the chat keeps running the model it
 * had before, because `sendPrompt` reads the store and the store had never heard of the choice. The
 * dropdown's `choose` is the same four facts, so it is what this calls.
 */
export async function chooseModel(modelId: string, providerId: string): Promise<boolean> {
  try {
    await sdcpCall('models.select', { modelId, providerId });

    const { catalog, choose } = useModelStore.getState();
    const row = catalog.find((model) => model.id === modelId && model.providerId === providerId);

    choose({
      engine: engineForProvider(providerId),
      providerId,
      model: modelId,
      /* The row's own tier when the catalogue listed it, and the tier in hand otherwise - a provider's
         live list can contain a model this build has no tier for. */
      tier: row?.tier ?? useModelStore.getState().tier,
    });

    return true;
  } catch (error) {
    reportFailure(error, strings.connect.modelFailed);
    return false;
  }
}

/* ------------------------------------------------------------------------------------------------
 * Sessions and hosts
 * ---------------------------------------------------------------------------------------------- */

/**
 * The empty chat a host already has, if any - what `+ New chat` should land on.
 *
 * The report was *"bar bar new open korle onk chat open hoi"*: every click ran `session.open`, so five
 * clicks left five chats - four of them empty rows the person then had to delete one at a time. A new
 * chat is *one* new chat, so the click re-uses the host's untouched one when there is one, and only
 * asks the daemon when there is not.
 *
 * A chat is **empty** when the log holds no turn for it, which is the same fact `Pane` uses to choose
 * between its empty state and the stream - not a title, and not a heuristic about how long ago it was
 * touched. The order is the sidebar's, so an idle host keeps answering with the same chat instead of
 * minting a new row every time.
 *
 * Pure, and exported, because this is a decision about two lists rather than about a store: the test in
 * `intents.test.ts` holds it against the shapes the daemon actually sends.
 */
export function emptySessionOn(
  hosts: readonly HostView[],
  turns: readonly Pick<TurnView, 'sessionId'>[],
  hostId: string,
  activeTab: string | null,
): string | null {
  const host = hosts.find((candidate) => candidate.id === hostId);

  if (host === undefined) {
    return null;
  }

  const empty = host.sessions.filter(
    (session) => !turns.some((turn) => turn.sessionId === session.id),
  );

  /* The chat the caret is already in wins - a click while an untouched chat is open then costs nothing
     at all - and otherwise the newest empty one, which is the last row the sidebar draws. */
  const focused = empty.find((session) => session.id === activeTab);

  return focused?.id ?? empty.at(-1)?.id ?? null;
}

/** Create an empty session on a host and focus its prompt (spec sections 7.3, 9.5). */
export async function newChatOnHost(hostId: string): Promise<string | null> {
  const state = useAppStore.getState();
  const existing = emptySessionOn(
    state.hosts,
    state.turns,
    hostId,
    usePrefsStore.getState().activeTab,
  );

  if (existing !== null) {
    toast(strings.sidebar.reusedChatOn(hostId));

    return existing;
  }

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

/**
 * The **one step a VPS needs after its key is pinned**: copy SDC's key onto it, with the password, once
 * (0.7.13).
 *
 * This is the missing half of the trust flow, and the reason a host could look "added but never
 * connected": the pin answers *is this the right machine*, and this answers *may SDC get in*. The daemon
 * does the work in `host.add` - adding the same `user@host` again reuses the row and, because the key is
 * already pinned, goes straight to `install_key` and the probe - so the window sends the address it
 * already shows (`user@host:8443`, which the daemon parses) and the password it just read.
 *
 * The password travels with this one call and is kept nowhere: not in the store, not in the event log,
 * and not in the sentence that comes back.
 */
export async function installHostKey(hostId: string, password: string): Promise<boolean> {
  const host = useAppStore.getState().hosts.find((candidate) => candidate.id === hostId);

  if (host === undefined || host.address === '') {
    return false;
  }

  const answer = await addHost({
    type: 'ssh',
    target: host.address,
    label: host.name,
    ...(password === '' ? {} : { password }),
  });

  return answer !== null && answer.hostId === hostId;
}

/**
 * Spec section 9.12. The daemon answers immediately and then *measures* the host.
 *
 * The answer is passed back rather than swallowed (0.7.13), because the dialog's second step needs the
 * `hostId`: `host.add` scans the machine's host key and, when it is not one SDC pinned, the host
 * arrives `untrusted` with its fingerprint - and trusting it is `host.trust`, which takes that id. The
 * two sentences this function says are about the *row*; whether the machine can be reached is the
 * daemon's next sentence, on the host's own line (a fact about a host does not belong in a toast that a
 * relaunch replays).
 */
export async function addHost(input: {
  type: 'local' | 'ssh';
  target?: string;
  label?: string;
  /**
   * The password for a VPS, when the user chooses to give one.
   *
   * It goes to the daemon with this one call and is kept nowhere - the daemon has no field to store it
   * in. Since 0.7.13 it is only *spent* after the host's key is pinned (`host.trust` re-sends it), so a
   * password never reaches a machine whose identity SDC has not been asked about.
   */
  password?: string;
}): Promise<{ hostId: string; reused: boolean } | null> {
  if (input.type === 'local') {
    toast(strings.addHost.localAlready);
    return { hostId: 'local', reused: true };
  }

  if (!input.target || input.target.trim() === '') {
    toast(strings.addHost.needTarget);
    return null;
  }

  const label = input.label?.trim() === '' || input.label === undefined ? input.target : input.label;

  try {
    const answer = await sdcpCall('host.add', input);

    /* Same `user@host` twice is one host, and saying so is the difference between a list and four
       copies of one row (0.7.0). */
    toast(answer.reused ? strings.addHost.alreadyThere(label) : strings.addHost.added(label));

    return answer;
  } catch (error) {
    reportFailure(error, 'Could not connect to that host');
    return null;
  }
}

/**
 * `host.key` - what a host presents **now** (0.7.13).
 *
 * Two callers, one call: a host that is `untrusted` and whose fingerprint this window never saw (a
 * relaunch, a second window) needs the value to show and to act on, and a host whose key **changed**
 * needs the fingerprint it presents *now* before `Re-pin` can mean anything. The daemon pushes the
 * answer as a `HostStatus` too, so the sidebar's dot and sentence move with it.
 */
export async function hostKey(
  hostId: string,
): Promise<{ hostKey: string; keyType: string; pinned: boolean; matches: boolean | null; pinnedKey: string | null } | null> {
  try {
    const answer = await sdcpCall('host.key', { hostId });

    return {
      hostKey: answer.hostKey,
      keyType: answer.keyType,
      pinned: answer.pinned,
      matches: answer.matches ?? null,
      pinnedKey: answer.pinnedKey ?? null,
    };
  } catch (error) {
    reportFailure(error, strings.addHost.trust.refused);

    return null;
  }
}

/**
 * `host.trust` - the answer to the one question a remote connection asks (0.7.13).
 *
 * The daemon scans the machine **again** and refuses if the key is no longer the fingerprint this was
 * called with, so trusting is a decision about a key that was on screen a moment ago rather than about
 * whatever answers the port now. The password, when the dialog still has it, is spent only after the pin
 * lands: it is what completes the one-time key install, and the install is what makes every later
 * connection passwordless.
 */
export async function trustHost(
  hostId: string,
  fingerprint: string,
  password?: string,
): Promise<boolean> {
  try {
    await sdcpCall('host.trust', {
      hostId,
      fingerprint,
      ...(password === undefined || password === '' ? {} : { password }),
    });

    toast(strings.addHost.trust.pinned(fingerprint));

    return true;
  } catch (error) {
    reportFailure(error, strings.addHost.trust.refused);

    return false;
  }
}

/**
 * Spec section 9.12's other half: take a host off the list.
 *
 * `host.remove` has been declared in `protocol/types.ts` since the schema was written and answered
 * `unknown method` until it was implemented, so a host added by mistake - or added three times under
 * the same name - could not be removed at all. The daemon deletes the row, its sessions and their
 * turns, and appends `HostRemoved`; that event is what takes the rows off this window's screen, and
 * off the next window's, because the log is what a reloading client replays.
 */
export async function removeHost(hostId: string, name: string): Promise<boolean> {
  try {
    const { sessions } = await sdcpCall('host.remove', { hostId });

    toast(
      sessions > 0
        ? strings.sidebar.hostRemovedWith(name, sessions)
        : strings.sidebar.hostRemoved(name),
    );

    return true;
  } catch (error) {
    reportFailure(error, 'Could not remove that host');
    return false;
  }
}

/**
 * `session.list`, folded into the store - what makes an added host survive a restart.
 *
 * Nothing used to ask for this. `host.add` wrote a row in the daemon's database and pushed one event;
 * the window folded that event and then lost the host the moment it was closed, and the next launch
 * showed only `local`. The host had not gone anywhere - nothing had asked for it.
 *
 * The `local` row is why `host.status` runs first: `session.list` answers from the daemon's rows, so
 * the machine this window is on has to be one of them (it is, because `host.status` records it).
 */
/**
 * Which host a chat's files live on (0.7.13).
 *
 * The window knows the chat; the daemon knows which machine the chat's folder is on - but only *per
 * method*: `fs.list` for a child path is sent without a session (the tree knows the directory, not the
 * chat), so the app names the host explicitly. `undefined` means "this machine", which is what the
 * daemon assumes when nothing is named - and it is what `local` means too.
 */
function hostIdOf(sessionId: string | null): string | undefined {
  if (sessionId === null) {
    return undefined;
  }

  const host = useAppStore
    .getState()
    .hosts.find((candidate) => candidate.sessions.some((session) => session.id === sessionId));

  return host === undefined || host.id === 'local' ? undefined : host.id;
}

/** A host's name, for a sentence. Falls back to the id so a stale reference still reads. */
function hostName(hostId: string): string {
  return useAppStore.getState().hosts.find((host) => host.id === hostId)?.name ?? hostId;
}

export async function loadWorkspace(): Promise<void> {
  try {
    const { hosts } = await sdcpCall('session.list', {});

    useAppStore.setState((state) => withWorkspace(state, hosts));
  } catch (error) {
    reportFailure(error, strings.daemon.offline);
  }

  /* The folders are the other half of "where am I": a chat's row says which project it is bound to, and
     `project.list` says what those rows point at - a root, a name, and how many chats are in each. */
  await loadProjects();
}

/* ------------------------------------------------------------------------------------------------
 * Folders: the directory a chat works in (0.7.6)
 * ---------------------------------------------------------------------------------------------- */

/* ------------------------------------------------------------------------------------------------
 * Files: the tree in the sidebar, and the file the Preview shows (0.7.7)
 * ---------------------------------------------------------------------------------------------- */

/**
 * Opens or closes a folder's row, reading it the first time it is opened.
 *
 * The read is lazy on purpose: `fs.list` is one level deep, so a project with a `node_modules` in it
 * costs one request per folder a person actually opens rather than a walk of the whole tree.
 */
export async function toggleDirectory(path: string): Promise<void> {
  const files = useFilesStore.getState();

  if (files.expanded.includes(path)) {
    files.setExpanded(path, false);

    return;
  }

  files.setExpanded(path, true);

  if (files.directories[path] === undefined) {
    await loadDirectory(path);
  }
}

/**
 * `fs.list` for one directory - or, with `null`, for the **chat's** folder.
 *
 * `null` is what the tree asks for at its root, and it is the same contract 0.7.6 gave `git.*` and
 * `fs.search`: the window knows the chat, the daemon knows the folder. The answer names the directory
 * it listed, so the tree can show the root's name without the app ever joining a path.
 */
export async function loadDirectory(path: string | null): Promise<void> {
  const sessionId = usePrefsStore.getState().activeTab;
  const hostId = hostIdOf(sessionId);

  if (path !== null) {
    useFilesStore.getState().startLoading(path);
  }

  try {
    const answer =
      path === null
        ? await sdcpCall('fs.list', { sessionId: sessionId ?? undefined, hostId })
        : await sdcpCall('fs.list', { path, hostId });
    const files = useFilesStore.getState();

    if (path === null) {
      files.setRoot(answer.path);
    }

    useFilesStore
      .getState()
      .fill(answer.path, { entries: answer.entries, hidden: answer.hidden });
  } catch (error) {
    reportFailure(error, strings.files.failed);
    useFilesStore.getState().fail(isSdcpError(error) ? error.message : strings.files.failed);
  }
}

/* ------------------------------------------------------------------------------------------------
 * The tree's own changes (v4): new file, new folder, rename, delete - and a search of the whole folder
 * ---------------------------------------------------------------------------------------------- */

/** The directory a path is in, with the path's own separator. */
function parentOf(path: string): string {
  const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));

  return cut <= 0 ? path : path.slice(0, cut);
}

/** Re-reads one directory of the tree and the git badge, after a change made from the tree. */
async function refreshAfter(directory: string): Promise<void> {
  const root = useFilesStore.getState().root;

  await loadDirectory(directory === root ? null : directory);
  await loadGitStatus();
}

/** A name a person typed for a file or folder: one path segment, nothing that climbs out. */
export function validName(name: string): boolean {
  const trimmed = name.trim();

  return trimmed !== '' && trimmed !== '.' && trimmed !== '..' && !/[\\/:*?"<>|]/.test(trimmed);
}

/** New file: an empty file, opened in a tab. `fs.write` checkpoints first, like every write. */
export async function createFile(directory: string, name: string): Promise<boolean> {
  const sessionId = usePrefsStore.getState().activeTab;

  if (sessionId === null || !validName(name)) {
    toast(strings.files.badName);

    return false;
  }

  const path = inFolder(directory, name.trim());

  try {
    await sdcpCall('fs.stat', { path, hostId: hostIdOf(sessionId) });
    toast(strings.files.exists(name.trim()));

    return false;
  } catch {
    /* Not there yet - which is what a new file needs. */
  }

  try {
    await sdcpCall('fs.write', { path, text: '', sessionId, hostId: hostIdOf(sessionId) });
    await refreshAfter(directory);
    await openFile(path, name.trim());

    return true;
  } catch (error) {
    reportFailure(error, strings.files.createFailed);

    return false;
  }
}

/** New folder. */
export async function createFolder(directory: string, name: string): Promise<boolean> {
  const sessionId = usePrefsStore.getState().activeTab;

  if (sessionId === null || !validName(name)) {
    toast(strings.files.badName);

    return false;
  }

  try {
    await sdcpCall('fs.mkdir', { path: inFolder(directory, name.trim()), sessionId, hostId: hostIdOf(sessionId) });
    await refreshAfter(directory);

    return true;
  } catch (error) {
    reportFailure(error, strings.files.createFailed);

    return false;
  }
}

/** Rename, in the same directory. Open tabs of the old path are closed - their path no longer exists. */
export async function renamePath(path: string, name: string): Promise<boolean> {
  const sessionId = usePrefsStore.getState().activeTab;

  if (sessionId === null || !validName(name)) {
    toast(strings.files.badName);

    return false;
  }

  const directory = parentOf(path);
  const to = inFolder(directory, name.trim());

  if (to === path) {
    return true;
  }

  try {
    await sdcpCall('fs.rename', { path, to, sessionId, hostId: hostIdOf(sessionId) });
    closeTabsUnder(path);
    await refreshAfter(directory);
    toast(strings.files.renamed(baseName(path), name.trim()));

    return true;
  } catch (error) {
    reportFailure(error, strings.files.renameFailed);

    return false;
  }
}

/** Delete - a checkpoint is taken first, so Rewind brings it back. */
export async function deletePath(path: string): Promise<boolean> {
  const sessionId = usePrefsStore.getState().activeTab;

  if (sessionId === null) {
    return false;
  }

  try {
    await sdcpCall('fs.delete', { path, sessionId, hostId: hostIdOf(sessionId) });
    closeTabsUnder(path);
    await refreshAfter(parentOf(path));
    toast(strings.files.deleted(baseName(path)));

    return true;
  } catch (error) {
    reportFailure(error, strings.files.deleteFailed);

    return false;
  }
}

/** Closes every open tab at or under a path that just moved or went away. */
function closeTabsUnder(path: string): void {
  const files = useFilesStore.getState();

  for (const tab of files.tabs) {
    if (tab.path === path || tab.path.startsWith(`${path}/`) || tab.path.startsWith(`${path}\\`)) {
      useFilesStore.getState().closeTab(tab.path);
    }
  }
}

/** One hit of a folder-wide search. */
export interface SearchHit {
  path: string;
  line: number;
  text: string;
}

/** A literal search of the chat's folder (`fs.search`), here or on the host. */
export async function searchFolder(query: string): Promise<SearchHit[] | null> {
  const sessionId = usePrefsStore.getState().activeTab;

  if (sessionId === null || query.trim() === '') {
    return [];
  }

  try {
    const { hits } = await sdcpCall('fs.search', { query: query.trim(), sessionId, hostId: hostIdOf(sessionId), limit: 200 });

    return hits.map((hit) => ({ path: hit.path, line: hit.line, text: hit.text }));
  } catch (error) {
    reportFailure(error, strings.files.searchFailed);

    return null;
  }
}

/**
 * `fs.read` for the file a row was clicked on, and then **show** it.
 *
 * Showing it means the Preview tab and the panel itself: a person who clicked a file in the tree
 * asked to see that file, and opening a tab behind a folded panel would be a click that looks like it
 * did nothing. The `truncated` flag travels with the text, so a megabyte of a larger file is never
 * mistaken for the whole of it.
 */
export async function openFile(path: string, name: string, line?: number): Promise<void> {
  const files = useFilesStore.getState();
  const already = files.tabs.find((tab) => tab.path === path);

  /* An open tab is brought forward rather than re-read: it may hold unsaved text. */
  if (already !== undefined) {
    files.activate(path);
    files.setReveal(line === undefined ? null : { path, line });
    useLayoutStore.getState().showRight();
    useRightPanelStore.getState().setActiveTab('preview', usePrefsStore.getState().activeTab);

    return;
  }

  files.startOpening(path);

  try {
    const answer = await sdcpCall('fs.read', {
      path,
      hostId: hostIdOf(usePrefsStore.getState().activeTab),
    });

    useFilesStore.getState().setOpen({
      path: answer.path,
      name,
      text: answer.text,
      sha256: answer.sha256,
      bytes: answer.bytes,
      truncated: answer.truncated,
    });
    useFilesStore.getState().setReveal(line === undefined ? null : { path: answer.path, line });

    useLayoutStore.getState().showRight();
    useRightPanelStore.getState().setActiveTab('preview', usePrefsStore.getState().activeTab);
  } catch (error) {
    reportFailure(error, strings.files.openFailed);
    useFilesStore.getState().fail(isSdcpError(error) ? error.message : strings.files.openFailed);
  }
}

/** Closes the open file: the Preview goes back to what it shows with nothing open. */
export function closeFile(): void {
  useFilesStore.getState().setOpen(null);
}

/** Re-reads a directory, so a file an engine just wrote shows up (the tree's Refresh). */
export async function refreshDirectory(path: string): Promise<void> {
  await loadDirectory(path);
  /* The same Refresh re-reads the branch and the changed count: a file an engine wrote changes both, and a
     badge that only updates when the folder changes is a badge that lies after the most interesting event. */
  await loadGitStatus();
}

/**
 * After a turn in this chat ends (v4): the tree, the git badge and the open files show what it changed.
 *
 * An agent edits files the window has on screen. Until this, the tree kept its old listing, the badge
 * kept saying `clean`, and an open tab kept the text from before the edit - so the window disagreed with
 * the disk right after the most interesting thing that happened to it. Every open directory is re-read,
 * and every open tab **without unsaved text** is re-read; a tab the person is editing is left alone (their
 * text is not overwritten), and the Save that follows is what decides.
 */
export async function refreshAfterTurn(): Promise<void> {
  const files = useFilesStore.getState();

  if (files.root === null) {
    return;
  }

  const hostId = hostIdOf(usePrefsStore.getState().activeTab);

  await loadDirectory(null);

  for (const directory of files.expanded) {
    await loadDirectory(directory);
  }

  await loadGitStatus();

  for (const tab of useFilesStore.getState().tabs) {
    if (useFilesStore.getState().drafts[tab.path] !== undefined) {
      continue;
    }

    try {
      const answer = await sdcpCall('fs.read', { path: tab.path, hostId });

      if (answer.sha256 !== tab.sha256) {
        const fresh = { ...tab, text: answer.text, sha256: answer.sha256, bytes: answer.bytes, truncated: answer.truncated };

        useFilesStore.setState((state) => ({
          tabs: state.tabs.map((candidate) => (candidate.path === tab.path ? fresh : candidate)),
          open: state.open?.path === tab.path ? fresh : state.open,
        }));
      }
    } catch {
      /* The file went away (deleted or renamed by the turn): its tab closes rather than lying. */
      useFilesStore.getState().closeTab(tab.path);
    }
  }
}

/**
 * Saves the open file (`fs.write`) - **and the daemon takes a checkpoint first** (principle P5).
 *
 * The checkpoint is not written here on purpose: a rule about not changing a file without a checkpoint
 * belongs where the file is changed, or the first caller that forgets it is a caller that silently skips it.
 * The window sends the chat's id and the daemon, which knows the folder (0.7.6), hashes the files before the
 * write and pushes `CheckpointSaved`. That is the same path `shell.run` has taken since 0.7.0, and it makes
 * "Save" the first *UI gesture* that honours the rule.
 *
 * The store is updated from what was saved rather than by re-reading: the text is known, and the daemon's
 * answer carries the new hash, so a second request would only tell us what we already have.
 */
export async function saveFile(path: string, text: string): Promise<boolean> {
  const sessionId = usePrefsStore.getState().activeTab;
  const hostId = hostIdOf(sessionId);
  const open = useFilesStore.getState().open;

  try {
    const answer = await sdcpCall('fs.write', {
      path,
      text,
      ...(sessionId === null ? {} : { sessionId }),
      hostId,
    });

    const saved = useFilesStore.getState().tabs.find((tab) => tab.path === path) ?? (open?.path === path ? open : null);

    if (saved !== null) {
      /* The tab takes what was written, and its draft is gone: the disk and the editor agree again. The
         active tab stays the active one - saving a background tab must not bring it forward. */
      useFilesStore.setState((state) => ({
        tabs: state.tabs.map((tab) => (tab.path === path ? { ...saved, text, sha256: answer.sha256, bytes: answer.bytes, truncated: false } : tab)),
        open: state.open?.path === path ? { ...saved, text, sha256: answer.sha256, bytes: answer.bytes, truncated: false } : state.open,
      }));
    }

    useFilesStore.getState().setDraft(path, null);

    /* A save on a host takes no checkpoint, and the sentence says which of the two happened: the shadow
       repository is on the machine `sdcd` runs on, and the file is not (docs/REMOTE.md §5). */
    toast(
      hostId === undefined
        ? strings.files.saved(nameOf(path))
        : strings.files.savedRemote(nameOf(path), hostName(hostId)),
    );

    /* The badge is live, not a snapshot: a Save is exactly the moment the changed count and the Diff button
       become interesting, and reading `git.status` once when the folder was opened left both stale (found by
       the probe, which saved a line and watched `0 changed` stay `0`). */
    await loadGitStatus();

    return true;
  } catch (error) {
    reportFailure(error, strings.files.saveFailed);

    return false;
  }
}

/**
 * `git.status` for the chat's folder - the branch and how many files the working tree has changed.
 *
 * Read for the Files header, next to the folder's name: "which branch, and has anything changed" is the
 * question a person asks right after "which folder am I in", and both come from the daemon (neither is
 * guessed from the machine the window happens to run on).
 */
export async function loadGitStatus(): Promise<void> {
  const sessionId = usePrefsStore.getState().activeTab;

  if (sessionId === null) {
    useFilesStore.getState().setGit(null);

    return;
  }

  try {
    const answer = await sdcpCall('git.status', {
      sessionId,
      hostId: hostIdOf(sessionId),
      ...(useFilesStore.getState().root === null ? {} : { root: useFilesStore.getState().root ?? undefined }),
    });

    /* An empty branch name is the daemon saying "this folder is not a repository": no badge, and no invented
       `master` from the shadow repository's own `git init` - which is what the 0.7.9 probe caught, with a
       `master · clean` badge over a folder git knew nothing about. */
    useFilesStore
      .getState()
      .setGit(answer.branch === '' ? null : { branch: answer.branch, dirty: answer.dirty });
  } catch {
    /* Not a git folder, or no git on this machine: no badge, and no toast either. A folder without git is
       a normal folder, not a failure. */
    useFilesStore.getState().setGit(null);
  }
}

/** `git.diff` for the chat's folder, shown in the Preview behind the file view's Diff button. */
export async function openDiff(): Promise<boolean> {
  const sessionId = usePrefsStore.getState().activeTab;
  const root = useFilesStore.getState().root;

  try {
    const { patch } = await sdcpCall('git.diff', {
      sessionId: sessionId ?? undefined,
      hostId: hostIdOf(sessionId),
      ...(root === null ? {} : { root }),
    });

    useFilesStore.getState().setDiff(patch);
    useLayoutStore.getState().showRight();
    useRightPanelStore.getState().setActiveTab('preview', sessionId);

    return true;
  } catch (error) {
    reportFailure(error, strings.files.diffFailed);

    return false;
  }
}

/** Closes the diff, putting the file view (or the empty Preview) back. */
export function closeDiff(): void {
  useFilesStore.getState().setDiff(null);
}

/**
 * Spec section 9.14's fork: branch this chat into a new one, each keeping its own turns.
 *
 * `session.fork` has been in the schema and in `protocol/types.ts` since they were written, and the daemon
 * answered `unknown method` until 0.7.8 - so the button that the spec draws next to a session had nothing
 * behind it. The daemon copies the conversation's turns into the new chat and replays them into the log, so
 * the fork arrives on screen with its transcript rather than as an empty row.
 *
 * `title` is only for the sentence: the fork's own title is derived by the daemon (`<title> (fork)`), which
 * is where it can see the parent's title without a race.
 */
export async function forkSession(sessionId: string, title: string): Promise<string | null> {
  try {
    const { sessionId: forked, turns } = await sdcpCall('session.fork', { sessionId });

    toast(strings.sidebar.forked(title, turns));

    return forked;
  } catch (error) {
    reportFailure(error, 'Could not fork that chat');

    return null;
  }
}

/**
 * `project.list`, folded into the store - what makes an opened folder survive a reload.
 *
 * A read, so a state patch (`withProjects`) rather than a stream of events, for the same reason
 * `session.list` is one. Nothing asked for this list before 0.7.6 because nothing could be there to ask
 * for: the `projects` table and `sessions.project_id` had been in the schema since the first migration
 * with nothing writing either, so a chat had no working directory at all and the engines ran wherever the
 * daemon had been started.
 */
export async function loadProjects(): Promise<void> {
  try {
    const { projects } = await sdcpCall('project.list', {});

    useAppStore.setState((state) => withProjects(state, projects));
  } catch (error) {
    reportFailure(error, strings.daemon.offline);
  }
}

/**
 * `Open folder` - spec section 7.13's `No project` state, and the way out of it.
 *
 * The dialog is `lib/picker.ts`'s (the same `tauri-plugin-dialog` the paperclip uses, in `directory`
 * mode), so this is a real folder chooser and not a text field: a path typed by hand is a path that is
 * wrong about a separator, and the daemon then has to refuse it.
 *
 * `hostId` defaults to `local` because that is where this build's engines run - a folder on a VPS host is
 * a later step, and defaulting to `local` keeps the button honest until it is.
 */
export async function openFolder(hostId = 'local'): Promise<string | null> {
  let root: string | null;

  try {
    root = await pickFolder();
  } catch (error) {
    reportFailure(error, strings.prompt.pickFailed);

    return null;
  }

  return root === null ? null : openFolderIn(root, hostId);
}

/**
 * The half of `Open folder` that runs *after* something answered with a path.
 *
 * Split out so a folder can be opened by anything that is not the native dialog - a test, a drop, a future
 * `--folder` argument - and so the dialog and the daemon call can be tested separately.
 *
 * Three things happen, in this order, and the order is the point:
 *
 *   1. `project.add` - the daemon validates the path (`is_dir`) and answers with the id. Opening the same
 *      folder twice reuses its row instead of leaving two rows for one directory;
 *   2. the chat - the host's *empty* chat if it has one (the rule `+ New chat` has followed since 0.7.5,
 *      so opening a folder does not leave an orphan row behind), otherwise a new chat opened with
 *      `projectId`, so its very first event already carries the folder;
 *   3. landing in it - the tab opens and the prompt takes the caret, because a person who just chose a
 *      folder wants to type in it.
 */
export async function openFolderIn(root: string, hostId = 'local'): Promise<string | null> {
  let sessionId: string;
  let message: string;

  try {
    const { projectId, name } = await sdcpCall('project.add', { hostId, root });

    await loadProjects();

    const state = useAppStore.getState();
    const empty = emptySessionOn(
      state.hosts,
      state.turns,
      hostId,
      usePrefsStore.getState().activeTab,
    );

    if (empty !== null) {
      await sdcpCall('session.update', { sessionId: empty, projectId });

      sessionId = empty;
      message = strings.folder.pointed(name, titleOf(state, empty));
    } else {
      const opened = await sdcpCall('session.open', {
        hostId,
        projectId,
        title: name,
        prompt: strings.sidebar.sessions.newChat.prompt,
      });

      sessionId = opened.sessionId;
      message = strings.folder.opened(name);
    }
  } catch (error) {
    reportFailure(error, strings.folder.couldNotOpen);

    return null;
  }

  /* Said and focused *after* the daemon's work, so a focus that cannot happen is never reported as a
     folder that could not open. */
  toast(message);
  landIn(sessionId);

  return sessionId;
}

/**
 * `Change folder` on a chat that already exists: the same dialog, another destination.
 *
 * This is what the prompt area's folder chip calls. The daemon's `SessionUpdated` carries the new
 * `projectRoot`, so the chip follows without this function touching state.
 */
export async function changeFolder(sessionId: string): Promise<boolean> {
  const state = useAppStore.getState();
  const host = state.hosts.find((candidate) =>
    candidate.sessions.some((session) => session.id === sessionId),
  );
  let root: string | null;

  try {
    root = await pickFolder();
  } catch (error) {
    reportFailure(error, strings.prompt.pickFailed);

    return false;
  }

  if (root === null) {
    return false;
  }

  try {
    const { projectId, name } = await sdcpCall('project.add', { hostId: host?.id ?? 'local', root });

    await sdcpCall('session.update', { sessionId, projectId });
    await loadProjects();
    toast(strings.folder.changed(name));

    return true;
  } catch (error) {
    reportFailure(error, strings.folder.couldNotChange);

    return false;
  }
}

/**
 * `project.remove`: close the folder, keep the chats.
 *
 * The daemon unbinds every chat that was looking at it (`project_id` → NULL) and deletes the row; the
 * conversations stay, which is the whole reason this is not `session.close` in a loop. The workspace is
 * re-read afterwards, because the rows this window holds still point at a folder that no longer exists -
 * the same reason `loadWorkspace` exists at all.
 */
export async function closeFolder(projectId: string, name: string): Promise<boolean> {
  try {
    const { chats } = await sdcpCall('project.remove', { projectId });

    await loadWorkspace();

    /* The daemon toasts for itself when chats were unbound - it knows the count; the quiet case is ours. */
    if (chats === 0) {
      toast(strings.folder.closed(name, 0));
    }

    return true;
  } catch (error) {
    reportFailure(error, strings.folder.couldNotChange);

    return false;
  }
}

/**
 * `fs.list` on a **host**, for the folder browser (0.7.13).
 *
 * No session is involved: the browser is about a machine, not a chat - it reads `~/` first (the daemon
 * expands it and answers with the absolute path), then one level at a time.
 */
export async function listRemoteDirectory(
  hostId: string,
  path?: string,
): Promise<{ path: string; entries: FsEntry[]; hidden: number } | null> {
  try {
    return await sdcpCall('fs.list', { hostId, ...(path === undefined ? {} : { path }) });
  } catch (error) {
    reportFailure(error, strings.remoteFolder.readFailed);

    return null;
  }
}

/**
 * Opens a folder **on a host**: `project.add` with that `hostId`, which validates the path with
 * `test -d` on the machine that has it - and then the ordinary landing (`openFolderIn`), so a chat whose
 * folder is on a VPS is bound, opened and shown exactly like a local one.
 */
export async function openRemoteFolder(hostId: string, root: string): Promise<string | null> {
  return openFolderIn(root, hostId);
}

/** Opens the chat's tab and puts the caret in its prompt - where a person wants to be after a folder. */
function landIn(sessionId: string): void {
  usePrefsStore.getState().openTab(sessionId);

  /* A window-less context (the unit tests) has no DOM to focus; the tab is still open, which is the part
     that matters. Same guard as `lib/picker.ts` and `lib/sdcp.ts` use for the same reason. */
  if (typeof window === 'undefined') {
    return;
  }

  /* The pane renders a tick later; a macrotask is enough and does not depend on a frame clock. */
  window.setTimeout(() => {
    document.querySelector<HTMLTextAreaElement>('.prompt-box textarea')?.focus();
  }, 0);
}

/** A session's title, wherever it sits in the tree - for a sentence about it. */
function titleOf(state: AppState, sessionId: string): string {
  for (const host of state.hosts) {
    const found = host.sessions.find((session) => session.id === sessionId);

    if (found !== undefined) {
      return found.title;
    }
  }

  return sessionId;
}

/**
 * One heartbeat: is the daemon still answering?
 *
 * `event.subscribe` rather than `host.status`, and the difference is the log. `host.status` *pushes*
 * a `HostStatus` event, so a five-second heartbeat against it would append 720 events an hour to a
 * log whose entire purpose is to be a readable record of what happened. `event.subscribe` is a pure
 * read - it answers `fromSeq` and appends nothing - and subscribing is what a client asks for anyway.
 *
 * A failure here is the one failure the event log cannot describe: the log's writer is the thing
 * that stopped. So it lands in `store/daemon.ts`, which is presentation, not history.
 */
export async function heartbeat(): Promise<boolean> {
  const beat = useDaemonStore.getState();
  const wasOnline = beat.online;

  try {
    await sdcpCall('event.subscribe', {});

    beat.beat(true);

    if (!wasOnline) {
      toast(strings.daemon.backOnline);
    }

    return true;
  } catch (error) {
    beat.beat(false, isSdcpError(error) ? error.message : strings.daemon.offline);

    if (wasOnline) {
      toast(strings.daemon.lost);
    }

    return false;
  }
}

/** How often the window asks. Five seconds is fast enough to notice and slow enough to be free. */
const HEARTBEAT_MS = 5000;

/**
 * Starts the heartbeat and returns its stop function, so `App.tsx` can hold it in one effect.
 *
 * The first beat is immediate: a window that opens against a daemon which is not there should say so
 * at once rather than five seconds later.
 */
export function watchDaemon(): () => void {
  void heartbeat();

  const timer = window.setInterval(() => {
    void heartbeat();
  }, HEARTBEAT_MS);

  return () => window.clearInterval(timer);
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
  /**
   * The provider the model came from, when the app knows it.
   *
   * A model id from a provider's live list may be one this build's catalogue has never seen
   * (`deepseek-v4-pro`), and the daemon routes a native API turn by that id: with no provider it falls
   * back to the loopback `custom` endpoint and answers `No API key for custom` - for a provider that is
   * connected, with its key stored under its own entry. The provider is a fact the app has and the
   * daemon cannot derive, so it travels with the turn.
   */
  provider?: string;
  /** Agent mode, and how much the agent may do without asking (v4). */
  agent?: boolean;
  autonomy?: 'ask' | 'pro' | 'auto';
}

/** The app's mode as the agent's autonomy: Simple asks for everything, Pro for commands, Auto for danger. */
export function autonomyFor(mode: 'simple' | 'pro' | 'auto'): 'ask' | 'pro' | 'auto' {
  return mode === 'simple' ? 'ask' : mode;
}

/**
 * Sends one prompt: the prompt area's Send button, and the one path that makes the app *do* something.
 *
 * Until 0.5.0 the prompt area's Send was a toast - `Sent to claude_code · sonnet` - and no call left
 * the window. This is it done properly, in three steps that each say what they are doing:
 *
 *   1. **a session.** `engine.start` needs one. If the window has none open, `session.open` is called
 *      first (through `newChatOnHost`, so the tab the user sees is the session the daemon made);
 *   2. **the turn.** `engine.start` with the tier/engine/model the prompt area is showing. The daemon
 *      answers with a `turnId` and streams the rest as events - `TurnStarted` carries the prompt back,
 *      so the log holds both halves of the conversation;
 *   3. **the outcome.** `Sent to …`, or the daemon's own message. A refused call returns `null` and the
 *      caller puts the words back in the box.
 *
 * An engine that is not installed is not an error here: the daemon raises `ErrorRaised` with the
 * translator's plain sentence for it, which lands in the turn stream like any other event.
 */
export async function sendPrompt(prompt: string, target?: string): Promise<string | null> {
  const { tier, engine, model, providerId, compose } = useModelStore.getState();

  /* The pane's own chat when it says which (split view has two boxes), else the active one. */
  let sessionId = target ?? selectActiveSession()?.session.id ?? null;

  if (sessionId === null) {
    sessionId = await newChatOnHost(usePrefsStore.getState().activeHostId);
  }

  if (sessionId === null) {
    return null;
  }

  const turnId = await startTurn({
    sessionId,
    prompt,
    engine,
    model,
    tier: tierName(tier),
    ...(providerId === null ? {} : { provider: providerId }),
    agent: compose === 'agent',
    autonomy: autonomyFor(useLayoutStore.getState().mode),
  });

  if (turnId !== null) {
    toast(strings.prompt.sent(engine, model));
  }

  return turnId;
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

/* ------------------------------------------------------------------------------------------------
 * Verify (v4): the folder's checks, then a review by another engine
 * ---------------------------------------------------------------------------------------------- */

/** An engine that can review a change: one connected provider's strongest current model. */
export interface ReviewerOption {
  engine: string;
  model: string;
  provider: string;
  label: string;
}

/**
 * Every connected provider as a possible reviewer, the strongest model of each first in its group.
 *
 * The list is the model menu's own (`groupCatalog`: connected only, newest versions), so a reviewer that
 * would fail the moment it is asked is never offered.
 */
export function reviewerOptions(): ReviewerOption[] {
  const { catalog } = useModelStore.getState();
  const { groups } = groupCatalog(catalog, useAppStore.getState().providers);

  return groups.flatMap((group) => {
    const model = group.models[0];

    return model === undefined
      ? []
      : [{ engine: group.engine, model: model.id, provider: group.providerId, label: `${group.providerLabel} · ${model.name === '' ? model.id : model.name}` }];
  });
}

/**
 * The reviewer to use when the person has not picked one: **a different engine** than the one that wrote
 * the change, because a model reviewing its own work shares its own blind spots. The same engine with a
 * different model is the second choice; the same model is offered last, and only when nothing else is
 * connected.
 */
export function defaultReviewer(author: { engine: string; model: string } | null): ReviewerOption | null {
  const options = reviewerOptions();

  if (author === null) {
    return options[0] ?? null;
  }

  return (
    options.find((option) => option.engine !== author.engine) ??
    options.find((option) => option.model !== author.model) ??
    options[0] ??
    null
  );
}

/**
 * Runs Verify for a chat - for one turn when `turnId` is given (its own checkpoint and prompt travel
 * with the request), otherwise for the chat's newest turn.
 */
export async function runVerify(input: {
  sessionId: string;
  turnId?: string;
  reviewer: ReviewerOption | null;
  reviewFailing?: boolean;
}): Promise<string | null> {
  const state = useAppStore.getState();
  const turns = state.turns.filter((turn) => turn.sessionId === input.sessionId);
  const turn = input.turnId === undefined ? turns.at(-1) : turns.find((candidate) => candidate.id === input.turnId);
  /* The turn's first checkpoint: the review reads everything that changed after it. */
  const checkpoint =
    turn === undefined
      ? undefined
      : [...state.checkpoints].filter((entry) => entry.turnId === turn.id).sort((left, right) => left.turn - right.turn)[0];

  useLayoutStore.getState().showRight();
  useRightPanelStore.getState().setActiveTab('verify', input.sessionId);

  try {
    const { verifyId } = await sdcpCall('verify.run', {
      sessionId: input.sessionId,
      ...(turn === undefined ? {} : { turnId: turn.id, task: turn.prompt }),
      ...(checkpoint === undefined ? {} : { since: checkpoint.filesHash }),
      ...(input.reviewer === null
        ? {}
        : { reviewer: { engine: input.reviewer.engine, model: input.reviewer.model, provider: input.reviewer.provider } }),
      ...(input.reviewFailing === true ? { reviewFailing: true } : {}),
      hostId: hostIdOf(input.sessionId),
    });

    return verifyId;
  } catch (error) {
    reportFailure(error, strings.rightPanel.verify.failed);

    return null;
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
  const { providerId } = useModelStore.getState();

  await startTurn({
    sessionId: input.sessionId,
    prompt,
    engine: input.engine,
    model: input.model,
    tier: input.tier,
    /* The same fact the Send path sends: whoever runs this turn, the daemon needs to know which
       provider the model came from to find its endpoint and its key. */
    ...(providerId === null ? {} : { provider: providerId }),
    /* A fix is work, not a question: it always runs as an agent, at the window's autonomy. */
    agent: true,
    autonomy: autonomyFor(useLayoutStore.getState().mode),
  });
}


/* ------------------------------------------------------------------------------------------------
 * Terminal (the panel's command surface, 0.7.13)
 * ---------------------------------------------------------------------------------------------- */

/**
 * What a command typed into the Terminal is about: the active chat's folder, and the machine that
 * folder is on.
 *
 * The rule is the one every file and git call already follows - *the chat's folder on the chat's
 * machine* - and `where` is that rule as a sentence, because a terminal that does not say which machine
 * it is talking to is the most dangerous surface in a remote-capable app: `rm -rf build` looks the same
 * on a laptop and on production.
 */
export interface TerminalSubject {
  sessionId: string | null;
  /** The chat's folder, or `null` when it has none - then the command runs where the daemon runs. */
  root: string | null;
  /** `undefined` for this machine, which is also how the daemon spells "local". */
  hostId: string | undefined;
  where: string;
}

export function terminalSubject(): TerminalSubject {
  /* The same two reads the sessions facade makes (`store/sessions.ts` → `state.hosts`, `prefs.activeTab`),
     so the click and the render cannot disagree about which machine they mean. */
  return terminalSubjectFrom(useAppStore.getState().hosts, usePrefsStore.getState().activeTab);
}

/**
 * The pure half of [`terminalSubject`] - the statement the tab prints, from what it subscribes to.
 *
 * It exists because the panel keeps every tab mounted and hides the inactive ones with a class: the tab
 * is still on screen when the chat changes, and a store read inside a memo would leave `where` saying
 * `on prod-1` after you moved to a local chat. A function of `(hosts, activeTab)` gives the same sentence
 * to the click (which wants the truth *now*) and to the render (which must follow the subscription).
 */
export function terminalSubjectFrom(hosts: HostView[], activeTab: string | null): TerminalSubject {
  const found = activeTab === null ? null : findSession(hosts, activeTab);
  const sessionId = found?.session.id ?? null;
  const root = found?.session.projectRoot ?? null;
  const remote = found !== null && found.host.type !== 'local' ? found.host : null;
  const where =
    root === null
      ? strings.terminal.anywhere
      : remote === null
        ? root
        : strings.terminal.whereOn(root, remote.name);

  return { sessionId, root, hostId: remote?.id, where };
}

/** The parameters every terminal call sends: the folder, the chat and the host, each only when known. */
function terminalParams(subject: TerminalSubject): Record<string, unknown> {
  return {
    ...(subject.root === null ? {} : { root: subject.root }),
    ...(subject.sessionId === null ? {} : { sessionId: subject.sessionId }),
    ...(subject.hostId === undefined ? {} : { hostId: subject.hostId }),
  };
}

/**
 * A whole command line, run by the daemon's own shell - here, or in the chat's folder on its host.
 *
 * It goes through `shell.run`, so a typed command is a step in the conversation exactly as an engine's
 * `run` step is: the daemon writes a **checkpoint** first (the command may change files), announces the
 * tool call in the turn stream, and refuses a denied line with a sentence rather than a stack trace. The
 * refusal lands in the entry's `stderr`, because that is where a terminal shows the reason - and it is
 * toasted too, because the person asked for it a moment ago and is looking at the input, not the log.
 */
export async function runCommand(line: string): Promise<void> {
  const trimmed = line.trim();

  if (trimmed === '') {
    return;
  }

  const subject = terminalSubject();
  const terminal = useTerminalStore.getState();
  const id = terminal.start(trimmed, subject.where);

  terminal.remember(trimmed);
  terminal.setBusy(true);

  try {
    const answer = await sdcpCall('shell.run', { line: trimmed, ...terminalParams(subject) });

    useTerminalStore.getState().finish(id, {
      state: answer.ok ? 'done' : 'failed',
      stdout: answer.stdout ?? '',
      stderr: answer.stderr ?? '',
      code: answer.exitCode ?? null,
      ms: answer.durationMs ?? 0,
      timedOut: answer.timedOut === true,
    });
  } catch (error) {
    const sentence = isSdcpError(error) ? error.message : 'The command did not run';

    useTerminalStore.getState().finish(id, { state: 'failed', stderr: sentence, code: null });
    toast(sentence);
  } finally {
    useTerminalStore.getState().setBusy(false);
  }
}


/**
 * `Run in background`: a process that does not finish, whose output the tab reads while it runs.
 *
 * It is `pty.open`, which is the daemon's long-running form - and with a host it is the same call: the
 * daemon's child is an `ssh`, so `pty.output` reads the process's output on that machine and `pty.close`
 * signals its **process group** there. A dev server, a `tail -f`, a long build: none of them may hold
 * the input while they run, which is why this is a second button rather than a checkbox on `Run`.
 *
 * `line` is what a terminal has, so the daemon takes a line here too - the local platform's shell runs
 * it here, and the **host's** shell runs it there. One process at a time: the tab starts the next one
 * when the first has been stopped or has ended.
 */
export async function runInBackground(line: string): Promise<void> {
  const trimmed = line.trim();

  if (trimmed === '') {
    return;
  }

  const subject = terminalSubject();
  const terminal = useTerminalStore.getState();

  if (terminal.background !== null) {
    toast(strings.terminal.backgroundBusy);
    return;
  }

  const id = terminal.start(trimmed, subject.where);

  terminal.remember(trimmed);

  try {
    const answer = await sdcpCall('pty.open', {
      line: trimmed,
      /* `pty.open` names the folder `cwd` (its own contract since 0.7.0, and `command`+`args` still use
         it); `shell.run` names it `root`. Sending the wrong one is silent: the daemon would start the
         process in `$HOME` instead of the chat's folder, which for a remote chat is the wrong machine's
         home. */
      ...(subject.root === null ? {} : { cwd: subject.root }),
      ...(subject.sessionId === null ? {} : { sessionId: subject.sessionId }),
      ...(subject.hostId === undefined ? {} : { hostId: subject.hostId }),
    });

    useTerminalStore.getState().setBackground({
      id,
      ptyId: answer.ptyId,
      command: trimmed,
      where: subject.where,
    });
  } catch (error) {
    const sentence = isSdcpError(error) ? error.message : 'The process did not start';

    useTerminalStore.getState().finish(id, { state: 'failed', stderr: sentence, code: null });
    toast(sentence);
  }
}

/** One poll of the background process's output tail - `watchBackground` drives it. */
export async function pollBackground(): Promise<void> {
  const background = useTerminalStore.getState().background;

  if (background === null) {
    return;
  }

  try {
    const answer = await sdcpCall('pty.output', { ptyId: background.ptyId });
    const terminal = useTerminalStore.getState();

    terminal.update(background.id, {
      state: answer.state === 'running' ? 'running' : 'done',
      stdout: answer.lines.join('\n'),
      ms: answer.ms,
    });

    /* Ended on its own: the input is free again, and the output stays in the log. */
    if (answer.state !== 'running') {
      terminal.setBackground(null);
    }
  } catch (error) {
    /*
     * A daemon that restarted has no pty, and saying so beats polling a `not_found` for ever: the
     * process died with the daemon that started it, which is a fact rather than a poll failure.
     */
    if (isSdcpError(error) && error.code === 'not_found') {
      const terminal = useTerminalStore.getState();

      terminal.finish(background.id, { state: 'done', stderr: strings.terminal.ended });
      terminal.setBackground(null);

      return;
    }

    reportFailure(error, 'Could not read the output');
  }
}

/** `Stop`: the process's **group** on the host, or the child here. The daemon decides which. */
export async function stopBackground(): Promise<void> {
  const background = useTerminalStore.getState().background;

  if (background === null) {
    return;
  }

  try {
    await sdcpCall('pty.close', { ptyId: background.ptyId });
  } catch (error) {
    reportFailure(error, 'Could not stop it');
  }

  const terminal = useTerminalStore.getState();

  terminal.update(background.id, { state: 'done', stderr: strings.terminal.stopped });
  terminal.setBackground(null);
}

/** How often the background tail is read: fast enough to feel live, slow enough to be free. */
const BACKGROUND_POLL_MS = 1000;

/**
 * Starts the background poll and returns its stop function, the way `watchDaemon` does.
 *
 * The poll is global rather than owned by the tab: a process keeps running while you look at the Preview
 * or another chat, and its output has to keep filling in - a tail that only advances while it is on
 * screen is a tail that lies about what it collected.
 */
export function watchBackground(): () => void {
  const timer = window.setInterval(() => {
    void pollBackground();
  }, BACKGROUND_POLL_MS);

  return () => window.clearInterval(timer);
}

/**
 * Opens the Terminal tab **about a host** - the destination of a doctor row's `Install` fix (0.7.13).
 *
 * The tab runs commands in the active chat's folder on the active chat's host, so focusing one of that
 * host's chats is not a nicety: without it the surface would open as a terminal about **another machine**,
 * which is the one mistake a remote-capable terminal must not make. A host with no chat has no folder to
 * run in, and the sentence says so rather than opening an empty tab pointed somewhere else.
 */
export function openTerminalForHost(hostId: string): void {
  const host = useAppStore.getState().hosts.find((candidate) => candidate.id === hostId);
  const sessionId = host?.sessions[0]?.id ?? null;

  if (host === undefined || sessionId === null) {
    toast(strings.terminal.noChat(hostId));

    return;
  }

  usePrefsStore.getState().focusTab(sessionId);
  useRightPanelStore.getState().setActiveTab('terminal', sessionId);
  useLayoutStore.getState().showRight();
  toast(strings.terminal.openForHost(host.name));
}

