import type { Notification, SdcpEvent } from '../../../protocol/types';
import type { AppEvent } from './types';

/**
 * The event bus and the append-only log (master spec sections 3.3 and 5.6).
 *
 * Two rules decide the shape of this file:
 *
 *   Append-only        an event is written once and never mutated or removed (principle P5,
 *                      section 5.6). `EventLog` therefore has `append`, `replay` and `subscribe`,
 *                      and deliberately has no `update` or `clear` - a correction is a new event.
 *   One source         every surface in the app is derived from this list. The React store folds
 *                      it (`reducer.ts`), the Time Machine reads the `CheckpointSaved` entries out
 *                      of it, and `event.list` on the daemon side hands the same list to a
 *                      reconnecting client.
 *
 * `seq` is assigned here rather than by callers, and it continues across a reload: the log is the
 * session's, the daemon's counter is the authority for the run. `ts` comes from the injected clock,
 * which is what keeps the reducer pure - the *event* carries the time, the reducer only reads it.
 */

/** A clock, so the log can be stamped in a test without `Date.now()` leaking into the reducer. */
export type Clock = () => string;

/** The default clock: RFC 3339 in UTC, which is what the schema asks for. */
export const systemClock: Clock = () => new Date().toISOString();

/** One event per listener call. Returns the unsubscribe function. */
export type EventListener = (event: AppEvent) => void;

export class EventLog {
  private events: AppEvent[] = [];
  private listeners = new Set<EventListener>();
  private clock: Clock;

  constructor(clock: Clock = systemClock) {
    this.clock = clock;
  }

  /** How many events have been appended. Also the next event's `seq`. */
  get length(): number {
    return this.events.length;
  }

  /** The highest sequence number in the log; 0 for an empty log. */
  get seq(): number {
    return this.events.at(-1)?.seq ?? 0;
  }

  /** The log, oldest first. Read-only to every caller: mutation happens through `append`. */
  get all(): readonly AppEvent[] {
    return this.events;
  }

  /**
   * Appends one event and notifies every subscriber.
   *
   * `seq` is `payload.seq` when the caller is replaying a daemon notification (the daemon's own
   * counter is authoritative and must survive the fold), and the log's next number otherwise.
   */
  append(event: SdcpEvent, payload: Partial<AppEvent> = {}): AppEvent {
    const entry: AppEvent = {
      seq: payload.seq ?? this.seq + 1,
      ts: payload.ts ?? this.clock(),
      event,
      sessionId: payload.sessionId ?? null,
      turnId: payload.turnId ?? null,
    };

    this.events.push(entry);

    for (const listener of this.listeners) {
      listener(entry);
    }

    return entry;
  }

  /** Folds a daemon notification into the log. This is the only inbound path from SDCP. */
  accept(notification: Notification): AppEvent {
    return this.append(notification.event, {
      seq: notification.seq,
      ts: notification.ts,
      sessionId: notification.sessionId ?? null,
      turnId: notification.turnId ?? null,
    });
  }

  /** Everything after `since` - what a reconnecting client asks for (spec section 5.6). */
  replay(since = 0): readonly AppEvent[] {
    return this.events.filter((entry) => entry.seq > since);
  }

  /**
   * True when a notification skips a sequence number.
   *
   * The log is the projection's only input, so a gap would mean the view is silently wrong. The
   * subscription code asks this, and a `true` answer is worth an `event.list` call with `since` -
   * not a quiet repaint (principle P4: never lie about what happened).
   */
  hasGap(nextSeq: number): boolean {
    return nextSeq > this.seq + 1;
  }

  subscribe(listener: EventListener): () => void {
    this.listeners.add(listener);

    return () => {
      this.listeners.delete(listener);
    };
  }

  /** A test seam: a fresh log with the same clock. The app's log is never reset at runtime. */
  static from(events: readonly AppEvent[], clock: Clock = systemClock): EventLog {
    const log = new EventLog(clock);

    for (const entry of events) {
      log.append(entry.event, entry);
    }

    return log;
  }
}

/** The app's log. One per window, created once, never cleared. */
export const eventLog = new EventLog();

/**
 * A deterministic clock for tests and for the seeded demo: an ISO string a fixed number of
 * milliseconds after a fixed epoch, incremented on every call. Pure, so a snapshot test of the
 * reducer's output is stable.
 */
export function stepClock(startMs = Date.parse('2026-09-20T14:02:00.000Z'), stepMs = 1000): Clock {
  let now = startMs;

  return () => {
    const stamp = new Date(now).toISOString();
    now += stepMs;
    return stamp;
  };
}
