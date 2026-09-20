import type {
  Envelope,
  MethodParams,
  MethodResult,
  Notification,
  Response,
  SdcpError,
  SdcpMethod,
  SdcpTransport,
} from '../../../protocol/types';

/**
 * The three transports, and the one rule that makes them interchangeable: **the envelope does not
 * change** (master spec section 3.1).
 *
 *   unix       `sdcd`'s socket on Linux/macOS (`$XDG_RUNTIME_DIR/sdc/sdcd-<port>.sock`)
 *   pipe       the same daemon's named pipe on Windows (`\\.\pipe\sdcd`)
 *   ws         a daemon on the far side of an SSH tunnel, for a VPS host
 *
 * The first two are *not* opened from JavaScript - a WebView cannot - so `TauriTransport` routes
 * through the Tauri bridge's `sdcp_call` command and its `sdcp://event` push. That keeps this module
 * free of platform conditionals: it is a map from a transport kind to an implementation, and the
 * only branch is "is there a Tauri bridge at all", which `lib/sdcp.ts` makes once.
 *
 * `LoopbackTransport` is the fourth implementation and the one the browser dev server and the tests
 * use: it talks to an in-process daemon (`lib/daemon.ts`) with no socket at all. It carries the same
 * envelope, so a UI flow proved against the loopback is a UI flow that works against sdcd - which is
 * exactly the property spec section 3.1 asks for.
 */

export class SdcpCallError extends Error implements SdcpError {
  readonly code: SdcpError['code'];
  readonly data?: Record<string, unknown>;
  readonly retryable: boolean;

  constructor(error: SdcpError) {
    super(error.message);
    this.name = 'SdcpCallError';
    this.code = error.code;
    this.data = error.data;
    this.retryable = error.retryable ?? false;
  }
}

export function isSdcpError(value: unknown): value is SdcpError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    'message' in value &&
    typeof (value as SdcpError).code === 'string'
  );
}

/** Correlation ids. Monotonic, so a response can never be matched to the wrong request. */
let nextRequestId = 1;

export function makeEnvelope<M extends SdcpMethod>(
  method: M,
  params: MethodParams<M>,
): Envelope<MethodParams<M>> {
  return { v: '0.1', id: `req-${nextRequestId++}`, method, params };
}

/**
 * The base every transport shares: a pending-request table keyed by envelope id, and the subscriber
 * list notifications fan out to. A concrete transport only has to move bytes.
 */
export abstract class BaseTransport implements SdcpTransport {
  abstract readonly kind: SdcpTransport['kind'];

  protected readonly pending = new Map<
    string,
    { resolve: (value: unknown) => void; reject: (error: unknown) => void }
  >();

  private readonly subscribers = new Set<(notification: Notification) => void>();

  abstract send(envelope: Envelope): void;

  subscribe(handler: (notification: Notification) => void): () => void {
    this.subscribers.add(handler);

    return () => {
      this.subscribers.delete(handler);
    };
  }

  request<M extends SdcpMethod>(method: M, params: MethodParams<M>): Promise<MethodResult<M>> {
    const envelope = makeEnvelope(method, params);

    return new Promise<MethodResult<M>>((resolve, reject) => {
      this.pending.set(envelope.id, { resolve: resolve as (value: unknown) => void, reject });
      this.send(envelope);
    });
  }

  /** A concrete transport calls this for every inbound line. */
  protected accept(response: Response | Notification): void {
    if ('event' in response) {
      for (const subscriber of this.subscribers) {
        subscriber(response as Notification);
      }

      return;
    }

    const waiter = this.pending.get(response.id);

    if (!waiter) {
      return;
    }

    this.pending.delete(response.id);

    if ('error' in response) {
      waiter.reject(new SdcpCallError(response.error));
    } else {
      waiter.resolve(response.result);
    }
  }

  close(): void {
    for (const waiter of this.pending.values()) {
      waiter.reject(new SdcpCallError({ code: 'internal', message: 'transport closed' }));
    }

    this.pending.clear();
  }
}

/**
 * The in-process transport. `daemon` answers envelopes and may push notifications asynchronously,
 * which is what makes an engine's stream real rather than a resolved promise.
 *
 * Every answer is delivered on a macrotask. That is not politeness: it is why a button that waits
 * for a call really shows its spinner for a frame, and why a test can assert the intermediate state
 * ("Testing…") instead of only the settled one.
 */
export interface LoopbackDaemon {
  handle(
    envelope: Envelope,
    push: (event: unknown, meta?: Partial<Notification>) => void,
    reply: (response: Response) => void,
  ): void;
}

export class LoopbackTransport extends BaseTransport {
  readonly kind = 'unix' as const;

  private sequence = 0;

  constructor(private readonly daemon: LoopbackDaemon) {
    super();
  }

  send(envelope: Envelope): void {
    window.setTimeout(() => {
      this.daemon.handle(
        envelope,
        (event, meta) => {
          this.sequence = meta?.seq ?? this.sequence + 1;

          this.accept({
            v: '0.1',
            seq: this.sequence,
            ts: meta?.ts ?? new Date().toISOString(),
            sessionId: meta?.sessionId ?? null,
            turnId: meta?.turnId ?? null,
            event,
          } as unknown as Notification);
        },
        (response) => this.accept(response),
      );
    }, 0);
  }
}

/**
 * The Tauri bridge. `invoke` is `@tauri-apps/api/core`'s and `listen` is the event API's; both are
 * typed structurally so this module does not import Tauri and stays testable outside a WebView.
 */
export interface TauriBridge {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: (event: string, handler: (payload: unknown) => void) => Promise<() => void>;
}

export class TauriTransport extends BaseTransport {
  readonly kind = 'pipe' as const;

  private unlisten: (() => void) | null = null;

  constructor(private readonly bridge: TauriBridge) {
    super();

    void bridge
      .listen('sdcp://event', (payload) => this.accept(payload as Notification))
      .then((stop) => {
        this.unlisten = stop;
      });
  }

  send(envelope: Envelope): void {
    void this.bridge
      .invoke('sdcp_call', { method: envelope.method, params: envelope.params, id: envelope.id })
      .then((response) => this.accept(response as Response))
      .catch((cause: unknown) => {
        const waiter = this.pending.get(envelope.id);

        this.pending.delete(envelope.id);
        waiter?.reject(
          isSdcpError(cause)
            ? new SdcpCallError(cause)
            : new SdcpCallError({ code: 'internal', message: String(cause) }),
        );
      });
  }

  override close(): void {
    this.unlisten?.();
    this.unlisten = null;
    super.close();
  }
}

/** The remote case: the same envelope over a WebSocket, which is what an SSH tunnel carries. */
export class WebSocketTransport extends BaseTransport {
  readonly kind = 'ws' as const;

  private socket: WebSocket | null = null;

  constructor(url: string) {
    super();
    this.socket = new WebSocket(url);
    this.socket.addEventListener('message', (message) => {
      this.accept(JSON.parse(String(message.data)) as Response | Notification);
    });
  }

  send(envelope: Envelope): void {
    this.socket?.send(JSON.stringify(envelope));
  }

  override close(): void {
    this.socket?.close();
    this.socket = null;
    super.close();
  }
}
