import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  MethodParams,
  MethodResult,
  Notification,
  SdcpMethod,
  SdcpTransport,
} from '../../../protocol/types';
import { eventLog } from '../store/events';
import { LoopbackTransport, TauriTransport, WebSocketTransport } from './transport';
import type { TauriBridge } from './transport';
import { StandInDaemon } from './standin';

/**
 * The typed SDCP client (master spec sections 5 and 3.1).
 *
 * `sdcpCall('provider.test', { id, key })` is the whole surface a component sees: the params are
 * checked against the method's contract, the answer is checked against its result, and a failure
 * arrives as a thrown `SdcpCallError` with a machine-readable code. No component ever touches a
 * transport.
 *
 * ## Which transport
 *
 * 1. `VITE_SDCP_URL` set          → a WebSocket, which is how a VPS host is reached over its SSH
 *                                  tunnel (spec section 3.1, remote).
 * 2. Tauri bridge available       → `sdcp_call` + the `sdcp://event` push, i.e. the daemon on this
 *                                  machine over its unix socket or Windows named pipe.
 * 3. otherwise                    → the in-process daemon, so `pnpm dev` and the headless UI
 *                                  verification both have something to talk to.
 *
 * All three carry the same envelope, which is the property spec section 3.1 exists to guarantee:
 * the app does not know or care that the second one is a pipe and the third one is a function call.
 */

let transport: SdcpTransport | null = null;
let unsubscribe: (() => void) | null = null;
/* Clears the catch-up window's timer (`getTransport`), so a closed window leaves no callback behind. */
let stopCatchUp: (() => void) | null = null;

/**
 * The Tauri bridge, or null in a plain browser. `@tauri-apps/api` is safe to *import* outside a
 * WebView - it only fails when a command is actually invoked - so presence is decided by the
 * global Tauri 2 injects before any script runs.
 */
function detectTauriBridge(): TauriBridge | null {
  if (!('__TAURI_INTERNALS__' in window)) {
    return null;
  }

  return {
    invoke: (command, args) => invoke(command, args),
    listen: (event, handler) => listen(event, (payload) => handler(payload.payload)),
  };
}

/** The transport, created on first use. */
export function getTransport(): SdcpTransport {
  if (transport) {
    return transport;
  }

  const remote = import.meta.env.VITE_SDCP_URL as string | undefined;
  const bridge = detectTauriBridge();

  if (remote) {
    transport = new WebSocketTransport(remote);
  } else if (bridge) {
    transport = new TauriTransport(bridge);
  } else {
    /* A tab with no bridge: the stand-in answers the protocol's shape and refuses the capabilities a
       browser does not have (`lib/standin.ts`). It is not a demo of the product - `pnpm dev` shows an
       honest empty app, and the desktop build is where the daemon does the work. */
    transport = new LoopbackTransport(new StandInDaemon());
  }

  /*
   * Every notification goes into the log, and the log is what the store folds (spec section 3.3).
   * The client therefore has exactly one job on the inbound path, and the reducer has no idea
   * whether the event came from Rust or from a `setTimeout`.
   *
   * One exception, and it is the reason this comment is longer than the code. The first connection
   * hands the window the daemon's whole backlog as notifications, which is what draws the session list
   * and the turn history - but a replayed `Toast` is not news. It is the same sentence the person
   * already read when it happened, and a log that happens to contain a few long ones (the `ssh`
   * refusals a failed host probe writes) put them over the Provider Hub at every single launch. So a
   * toast is the one event type the catch-up window drops. A toast raised by the UI is not affected:
   * it is dispatched locally and never arrives through the transport.
   */
  let catchingUp = true;
  const catchUpTimer = window.setTimeout(() => {
    catchingUp = false;
  }, 1500);

  /** How old a replayed toast has to be before it is history rather than news. */
  const TOAST_IS_HISTORY_MS = 30_000;

  unsubscribe = transport.subscribe((notification: Notification) => {
    if (notification.event.type === 'Toast' || notification.event.type === 'ToastDismissed') {
      /* The envelope carries the moment the daemon wrote the event, so "old" is a fact rather than a
         guess about how long the backlog takes: a notification about something that happened half a
         minute ago is the log talking, whether it arrives during the catch-up window or after it. The
         daemon replays the backlog in answer to the window's first calls, which is later than the window
         opened - a timer alone was not enough. */
      const age = Date.now() - Date.parse(notification.ts);

      if (catchingUp || !Number.isFinite(age) || age > TOAST_IS_HISTORY_MS) {
        return;
      }
    }

    eventLog.accept(notification);
  });

  stopCatchUp = () => {
    window.clearTimeout(catchUpTimer);
  };

  return transport;
}

/** One request. Rejects with `SdcpCallError` carrying the daemon's error code. */
export function sdcpCall<M extends SdcpMethod>(
  method: M,
  params: MethodParams<M>,
): Promise<MethodResult<M>> {
  return getTransport().request(method, params);
}

/** Subscribes to notifications directly - used by the verification harness, not by the UI. */
export function sdcpSubscribe(handler: (notification: Notification) => void): () => void {
  return getTransport().subscribe(handler);
}

/** Closes the transport and detaches it from the log. Called by a window teardown, never by a UI. */
export function disconnect(): void {
  unsubscribe?.();
  unsubscribe = null;
  stopCatchUp?.();
  stopCatchUp = null;
  transport?.close();
  transport = null;
}
