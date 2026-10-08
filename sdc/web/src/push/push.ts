// Notifications for a page that is not open (plan 5.10): the browser's push service wakes the service worker, which shows
// "SDC needs you" and a link. Nothing but that crosses the push service; the details are on the computer, behind the passkey.
//
// Everything the browser provides is behind `PushEnv`, so the logic runs in tests with a fake one.

export type PushState =
  | { kind: 'checking' }
  /** No push in this browser. On an iPhone that usually means the page is not on the Home Screen yet. */
  | { kind: 'unsupported'; ios: boolean }
  | { kind: 'denied' }
  | { kind: 'off' }
  | { kind: 'on' }
  | { kind: 'working' }
  | { kind: 'failed'; why: string };

export interface Subscription {
  toJSON(): { endpoint?: string; keys?: { p256dh?: string; auth?: string } };
  unsubscribe(): Promise<boolean>;
}

export interface Registration {
  pushManager: {
    getSubscription(): Promise<Subscription | null>;
    subscribe(options: { userVisibleOnly: boolean; applicationServerKey: Uint8Array }): Promise<Subscription>;
  };
}

export interface PushEnv {
  supported(): boolean;
  /** iPhone/iPad, where push needs the page on the Home Screen. */
  ios(): boolean;
  permission(): 'default' | 'granted' | 'denied';
  requestPermission(): Promise<'default' | 'granted' | 'denied'>;
  /** Registers `/sw.js` (once) and resolves with the active registration. */
  registration(): Promise<Registration>;
  /** The relay's public VAPID key, or null if push is not set up there. */
  serverKey(): Promise<string | null>;
}

/** What the relay is told: an address and two keys. */
export interface RelayPush {
  registerPush(subscription: { endpoint: string; p256dh: string; auth: string } | null): Promise<void>;
}

export function base64UrlToBytes(text: string): Uint8Array {
  const padded = text.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (text.length % 4)) % 4);
  const binary = atob(padded);

  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}

function plain(subscription: Subscription): { endpoint: string; p256dh: string; auth: string } | null {
  const json = subscription.toJSON();

  return json.endpoint && json.keys?.p256dh && json.keys.auth ? { endpoint: json.endpoint, p256dh: json.keys.p256dh, auth: json.keys.auth } : null;
}

export class PushControl {
  state: PushState = { kind: 'checking' };

  constructor(
    private readonly env: PushEnv,
    private readonly relay: () => RelayPush | null,
    private readonly changed: () => void,
  ) {}

  private set(state: PushState): void {
    this.state = state;
    this.changed();
  }

  /** Works out where things stand, and (when notifications are already on) tells the relay again: an address can change. */
  async check(): Promise<void> {
    if (!this.env.supported()) return this.set({ kind: 'unsupported', ios: this.env.ios() });
    if (this.env.permission() === 'denied') return this.set({ kind: 'denied' });
    if (this.env.permission() !== 'granted') return this.set({ kind: 'off' });

    try {
      const existing = await (await this.env.registration()).pushManager.getSubscription();
      const sub = existing && plain(existing);

      if (!sub) return this.set({ kind: 'off' });

      this.set({ kind: 'on' });
      await this.relay()?.registerPush(sub).catch(() => undefined);
    } catch {
      this.set({ kind: 'off' });
    }
  }

  /** Must be called from a tap: browsers only show the permission prompt for a gesture. */
  async enable(): Promise<void> {
    if (!this.env.supported()) return this.set({ kind: 'unsupported', ios: this.env.ios() });

    this.set({ kind: 'working' });

    try {
      if ((await this.env.requestPermission()) !== 'granted') return this.set({ kind: 'denied' });

      const key = await this.env.serverKey();

      if (!key) return this.set({ kind: 'failed', why: 'Notifications are not set up on the relay yet.' });

      const registration = await this.env.registration();
      const subscription = (await registration.pushManager.getSubscription()) ?? (await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: base64UrlToBytes(key) }));
      const sub = plain(subscription);
      const relay = this.relay();

      if (!sub) throw new Error('the browser gave no usable subscription');
      if (!relay) throw new Error('not connected to your computer right now');

      await relay.registerPush(sub);
      this.set({ kind: 'on' });
    } catch (error) {
      this.set({ kind: 'failed', why: (error as Error).message });
    }
  }

  async disable(): Promise<void> {
    this.set({ kind: 'working' });

    try {
      const subscription = await (await this.env.registration()).pushManager.getSubscription();

      await this.relay()?.registerPush(null).catch(() => undefined);
      await subscription?.unsubscribe();
      this.set({ kind: 'off' });
    } catch (error) {
      this.set({ kind: 'failed', why: (error as Error).message });
    }
  }
}

/** The real browser. Constructed lazily so importing this file in a test (no `window`) is harmless. */
export function browserPushEnv(): PushEnv {
  const ready = (): boolean => typeof window !== 'undefined' && 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window;

  return {
    supported: ready,
    ios: () => typeof navigator !== 'undefined' && /iPhone|iPad|iPod/.test(navigator.userAgent),
    permission: () => (typeof Notification === 'undefined' ? 'denied' : Notification.permission),
    requestPermission: () => Notification.requestPermission(),
    registration: async () => {
      await navigator.serviceWorker.register('/sw.js', { scope: '/' });

      return (await navigator.serviceWorker.ready) as unknown as Registration;
    },
    serverKey: async () => {
      const response = await fetch('/api/push/key', { cache: 'no-store' });

      return response.ok ? String(((await response.json()) as { key?: string }).key ?? '') || null : null;
    },
  };
}
