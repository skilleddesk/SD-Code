import { describe, expect, it } from 'vitest';
import { PushControl, base64UrlToBytes, type PushEnv, type Registration, type RelayPush, type Subscription } from '../src/push/push';

const KEY = 'BKvi_9CXLPzwvJORYASL9Ma6AzgEeBBb5ydVnhpcq99nM_EBjK2ml10egS05bkDcKcNK_W-MvSC_ubFuoxLlZ-0';

function subscription(endpoint = 'https://fcm.googleapis.com/fcm/send/abc'): Subscription & { gone: boolean } {
  return {
    gone: false,
    toJSON: () => ({ endpoint, keys: { p256dh: 'P'.repeat(87), auth: 'A'.repeat(22) } }),
    async unsubscribe() {
      this.gone = true;

      return true;
    },
  };
}

function fakes(options: { supported?: boolean; ios?: boolean; permission?: 'default' | 'granted' | 'denied'; ask?: 'granted' | 'denied'; key?: string | null; existing?: Subscription | null } = {}) {
  const log: string[] = [];
  let current = options.existing ?? null;
  const registration: Registration = {
    pushManager: {
      async getSubscription() {
        return current;
      },
      async subscribe(opts) {
        log.push(`subscribe visible=${opts.userVisibleOnly} keyBytes=${opts.applicationServerKey.length}`);
        current = subscription();

        return current;
      },
    },
  };
  const env: PushEnv = {
    supported: () => options.supported ?? true,
    ios: () => options.ios ?? false,
    permission: () => options.permission ?? 'default',
    requestPermission: async () => {
      log.push('asked permission');

      return options.ask ?? 'granted';
    },
    registration: async () => registration,
    serverKey: async () => (options.key === undefined ? KEY : options.key),
  };
  const sent: Array<unknown> = [];
  let fail: string | null = null;
  const relay: RelayPush = {
    async registerPush(sub) {
      if (fail) throw new Error(fail);
      sent.push(sub);
    },
  };

  return { env, relay, log, sent, failRelay: (why: string | null) => void (fail = why), current: () => current };
}

function control(f: ReturnType<typeof fakes>, connected = true) {
  const states: string[] = [];
  const pc = new PushControl(f.env, () => (connected ? f.relay : null), () => states.push(pc.state.kind));

  return { pc, states };
}

describe('base64url', () => {
  it('decodes the VAPID key to the 65 bytes of an uncompressed point', () => {
    const bytes = base64UrlToBytes(KEY);

    expect(bytes.length).toBe(65);
    expect(bytes[0]).toBe(4);
  });
});

describe('turning notifications on', () => {
  it('asks only when tapped, subscribes with the relay key, and tells the relay', async () => {
    const f = fakes();
    const { pc } = control(f);

    await pc.check();
    expect(pc.state).toEqual({ kind: 'off' });
    expect(f.log).toEqual([]);

    await pc.enable();

    expect(pc.state).toEqual({ kind: 'on' });
    expect(f.log).toEqual(['asked permission', 'subscribe visible=true keyBytes=65']);
    expect(f.sent).toEqual([{ endpoint: 'https://fcm.googleapis.com/fcm/send/abc', p256dh: 'P'.repeat(87), auth: 'A'.repeat(22) }]);
  });

  it('a refused permission is reported, and nothing is subscribed or sent', async () => {
    const f = fakes({ ask: 'denied' });
    const { pc } = control(f);

    await pc.enable();

    expect(pc.state).toEqual({ kind: 'denied' });
    expect(f.log).toEqual(['asked permission']);
    expect(f.sent).toEqual([]);
  });

  it('says so when the relay has no push key, instead of subscribing to nothing', async () => {
    const f = fakes({ key: null });
    const { pc } = control(f);

    await pc.enable();

    expect(pc.state.kind).toBe('failed');
    expect(f.log).toEqual(['asked permission']);
  });

  it('reports a relay that refuses, or no connection, as a failure the person can read', async () => {
    const refusing = fakes();

    refusing.failRelay('that is not a browser push subscription');

    const one = control(refusing);

    await one.pc.enable();
    expect(one.pc.state).toEqual({ kind: 'failed', why: 'that is not a browser push subscription' });

    const offline = control(fakes(), false);

    await offline.pc.enable();
    expect(offline.pc.state).toMatchObject({ kind: 'failed', why: expect.stringContaining('not connected') });
  });

  it('explains an unsupported browser, with the Home Screen hint on an iPhone', async () => {
    const phone = control(fakes({ supported: false, ios: true }));
    const other = control(fakes({ supported: false }));

    await phone.pc.check();
    await other.pc.check();

    expect(phone.pc.state).toEqual({ kind: 'unsupported', ios: true });
    expect(other.pc.state).toEqual({ kind: 'unsupported', ios: false });
  });

  it('shows blocked notifications as blocked', async () => {
    const { pc } = control(fakes({ permission: 'denied' }));

    await pc.check();
    expect(pc.state).toEqual({ kind: 'denied' });
  });
});

describe('when they are already on', () => {
  it('a new visit shows them on and gives the relay the current address again', async () => {
    const f = fakes({ permission: 'granted', existing: subscription('https://fcm.googleapis.com/fcm/send/rotated') });
    const { pc } = control(f);

    await pc.check();

    expect(pc.state).toEqual({ kind: 'on' });
    expect(f.sent).toEqual([{ endpoint: 'https://fcm.googleapis.com/fcm/send/rotated', p256dh: 'P'.repeat(87), auth: 'A'.repeat(22) }]);
  });

  it('permission without a subscription is shown as off', async () => {
    const { pc } = control(fakes({ permission: 'granted', existing: null }));

    await pc.check();
    expect(pc.state).toEqual({ kind: 'off' });
  });

  it('a relay that cannot be reached at check time does not turn them off', async () => {
    const f = fakes({ permission: 'granted', existing: subscription() });

    f.failRelay('down');

    const { pc } = control(f);

    await pc.check();
    expect(pc.state).toEqual({ kind: 'on' });
  });

  it('turning off tells the relay and drops the browser subscription', async () => {
    const existing = subscription();
    const f = fakes({ permission: 'granted', existing });
    const { pc } = control(f);

    await pc.disable();

    expect(pc.state).toEqual({ kind: 'off' });
    expect(f.sent).toEqual([null]);
    expect(existing.gone).toBe(true);
  });
});
