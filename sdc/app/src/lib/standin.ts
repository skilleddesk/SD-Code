import type { Envelope, ErrorCode, Notification, Response, SdcpEvent } from '../../../protocol/types';
import catalogue from '../../../protocol/models.json';
import { strings } from '../strings';
import type { LoopbackDaemon } from './transport';

/**
 * The in-process **stand-in** for the browser.
 *
 * Earlier versions of this file were an in-process *daemon*: it invented three hosts, six chats, nine
 * providers, twelve models, a streaming answer with `TurnDelta`s, tool cards, checkpoints and a duel.
 * Every one of those was fiction, and the app could not tell it from work - which is precisely how a
 * window came to look connected and busy while nothing was running. It is gone.
 *
 * What is left is honest. It answers the protocol's *shape*, so `pnpm dev` still exercises the same
 * components and the same event log, and it refuses the calls a browser tab cannot possibly serve: a
 * tab has no child processes, so it has no engines, no CLIs and no filesystem. Instead of a plausible
 * lie it returns an empty result or an error that says why.
 *
 * The bundled model catalogue *is* real data that ships with the build, so `models.list` answers with
 * it and marks every row `bundled`, and says in `notes` that a provider's own list needs a daemon.
 *
 * The desktop app never constructs this: `lib/sdcp.ts` finds the Tauri bridge and uses the real
 * transport. `sdcd` is the peer that can actually do the work.
 */

type Push = (event: SdcpEvent, meta?: Partial<Notification>) => void;
type Reply = (response: Response) => void;
type Params = Record<string, unknown>;

/** One session the tab made, kept only while the tab lives. */
interface StandInSession {
  id: string;
  title: string;
  state: 'idle' | 'running' | 'waiting' | 'success' | 'error';
}

const text = (params: Params, key: string, fallback = ''): string =>
  typeof params[key] === 'string' ? (params[key] as string) : fallback;

const ok = (reply: Reply, envelope: Envelope, result: Record<string, unknown>): void => {
  reply({ v: '0.1', id: envelope.id, result });
};

const fail = (reply: Reply, envelope: Envelope, code: ErrorCode, message: string): void => {
  reply({ v: '0.1', id: envelope.id, error: { code, message } });
};

/**
 * Why a call cannot be served here.
 *
 * One sentence, and it names the thing to do about it, because the person reading it is looking at an
 * app that did nothing - not reading a stack trace.
 */
const NEEDS_DAEMON =
  'This is a browser tab: it has no daemon behind it, so it cannot run an engine, read the filesystem or hold a credential. Start the desktop app (which starts `sdcd` itself) and this becomes the real thing.';

export class StandInDaemon implements LoopbackDaemon {
  private sequence = 0;
  private readonly sessions: StandInSession[] = [];
  /** Set by `host.remove`: a tab's host list is one row, and removing it is the whole list. */
  private hostRemoved = false;

  handle(envelope: Envelope, push: Push, reply: Reply): void {
    const params = (envelope.params ?? {}) as Params;

    /* Notifications the tab *can* honestly produce carry the sequence the app correlates on. */
    const emit = (event: SdcpEvent, sessionId: string | null = null): void => {
      this.sequence += 1;
      push(event, { seq: this.sequence, sessionId });
    };

    switch (envelope.method) {
      case 'host.status': {
        const platform = typeof navigator === 'undefined' ? 'browser' : navigator.userAgent.slice(0, 60);

        emit({
          type: 'HostStatus',
          hostId: 'browser',
          name: strings.daemon.browserHostName,
          hostType: 'local',
          status: 'degraded',
          sdcd: '',
          platform,
        });

        ok(reply, envelope, {
          hostId: 'browser',
          name: strings.daemon.browserHostName,
          type: 'local',
          status: 'degraded',
          sdcd: '',
          sdcp: '0.1',
          platform,
          database: '',
          events: this.sequence,
          sessions: this.sessions.length,
          keychain: 'none',
          engines: [],
          pty: 0,
          console: 0,
          subscribers: 0,
          note: NEEDS_DAEMON,
        });
        return;
      }

      case 'session.open': {
        const id = `tab-${this.sessions.length + 1}`;
        const title = text(params, 'title', strings.daemon.browserSessionTitle);

        this.sessions.push({ id, title, state: 'idle' });

        emit({ type: 'SessionOpened', sessionId: id, title, hostId: 'browser', prompt: '' }, id);

        ok(reply, envelope, { sessionId: id });
        return;
      }

      case 'session.update': {
        const id = text(params, 'sessionId');
        const title = params['title'];

        if (typeof title === 'string') {
          const session = this.sessions.find((candidate) => candidate.id === id);

          if (session) {
            session.title = title;
          }

          emit({ type: 'SessionUpdated', sessionId: id, title }, id);
        }

        ok(reply, envelope, {});
        return;
      }

      case 'session.close': {
        const id = text(params, 'sessionId');
        const at = this.sessions.findIndex((candidate) => candidate.id === id);

        if (at >= 0) {
          this.sessions.splice(at, 1);
        }

        emit({ type: 'SessionClosed', sessionId: id }, id);
        ok(reply, envelope, {});
        return;
      }

      case 'session.list':
        ok(reply, envelope, {
          hosts: this.hostRemoved
            ? []
            : [
                {
                  hostId: 'browser',
                  name: strings.daemon.browserHostName,
                  hostType: 'local',
                  status: 'degraded',
                  platform: null,
                  target: null,
                  sessions: this.sessions.map((session) => ({
                    sessionId: session.id,
                    hostId: 'browser',
                    title: session.title,
                    prompt: '',
                    state: session.state,
                    unread: 0,
                    minutesAgo: 0,
                  })),
                },
              ],
        });
        return;

      /*
       * A tab has no host table, so the honest thing `host.remove` can do is the thing the real
       * daemon does to its own list: forget the host and say so in the log.
       */
      case 'host.remove': {
        const id = text(params, 'hostId');

        this.hostRemoved = true;

        emit({ type: 'HostRemoved', hostId: id, name: strings.daemon.browserHostName, sessions: 0 });
        ok(reply, envelope, { removed: true, name: strings.daemon.browserHostName, sessions: 0 });
        return;
      }

      /* The catalogue is data this build ships, so it is real: what is missing is the provider's live
         answer, which only a daemon can fetch. */
      case 'models.list': {
        const rows = bundledModels();
        const providerId = text(params, 'providerId');

        ok(reply, envelope, {
          models: providerId === '' ? rows : rows.filter((row) => row['providerId'] === providerId),
          snapshot: strings.daemon.bundledSnapshot,
          refreshed: false,
          selected: { providerId, modelId: '' },
          notes: [strings.daemon.bundledNote],
        });
        return;
      }

      case 'provider.list':
        ok(reply, envelope, { providers: [] });
        return;

      case 'provider.registry.list':
        ok(reply, envelope, { models: [] });
        return;

      case 'host.doctor':
        ok(reply, envelope, { checks: [] });
        return;

      case 'event.list':
        ok(reply, envelope, { events: [] });
        return;

      case 'event.append':
      case 'event.subscribe':
        ok(reply, envelope, {});
        return;

      default:
        /* Everything else is a capability: a child process, a socket, a keychain, a file. A tab has
           none of them, and saying so is the whole point of this file. */
        fail(reply, envelope, 'unsupported', NEEDS_DAEMON);
    }
  }
}

/**
 * The bundled catalogue, in the shape `models.list` answers with.
 *
 * `protocol/models.json` is the one list here that is not invented: it is the file this build ships.
 * The rows say so (`source: bundled`), and the note beside them says a provider's own answer needs a
 * daemon - which is the honest split.
 */
function bundledModels(): Record<string, unknown>[] {
  /* Both groups, because `models.list` serves both: the seven API blocks and the three subscription
     blocks (`sonnet`, `opus`, `haiku` on the Claude plan). A menu that showed only the API half would
     hide the models the user's plan actually has. */
  const groups = [...catalogue.providers, ...(catalogue.subscriptions ?? [])];

  return groups.flatMap((provider) =>
    provider.models.map((model) => ({
      id: model.id,
      name: 'name' in model && typeof model.name === 'string' ? model.name : model.id,
      providerId: provider.id,
      providerLabel: provider.label,
      tier: model.tier,
      ctx: model.ctx,
      cost: model.cost,
      source: 'bundled',
      fetchedAt: null,
      notes: [strings.daemon.bundledNote],
    })),
  );
}
