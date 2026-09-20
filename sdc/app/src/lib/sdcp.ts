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
import { DemoDaemon } from './daemon';

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
    transport = new LoopbackTransport(new DemoDaemon());
  }

  /*
   * Every notification goes into the log, and the log is what the store folds (spec section 3.3).
   * The client therefore has exactly one job on the inbound path, and the reducer has no idea
   * whether the event came from Rust or from a `setTimeout`.
   */
  unsubscribe = transport.subscribe((notification: Notification) => {
    eventLog.accept(notification);
  });

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
  transport?.close();
  transport = null;
}
