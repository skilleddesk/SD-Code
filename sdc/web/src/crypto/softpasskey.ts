// A software passkey for tests and the end-to-end check. It signs the way an authenticator does (WebAuthn
// section 6.1 authenticator data, 7.2 assertion), so the daemon's verifier is exercised with bytes it did
// not write. It is never used by the app itself: `BrowserPasskey` is.
//
// It also models the two things a passkey adds for rejoining: a user handle that is handed back on a discoverable sign-in, and a
// per-credential secret behind the PRF extension (HMAC-SHA-256 of the salt, as the real thing is specified to behave).

import { b64u, concat, fromB64u, sha256, utf8, type Bytes } from './bytes';
import { derFromRaw, rawKeyFromSpki, type AssertionWire, type Passkey } from './passkey';

interface Credential {
  privateKey: CryptoKey;
  counter: number;
  userHandle: Bytes;
  prfSecret: CryptoKey;
}

export class SoftPasskey implements Passkey {
  private readonly credentials = new Map<string, Credential>();
  /** Set to false to model a passkey with no PRF (older phones, some security keys). */
  public prf = true;

  constructor(
    private readonly rpId = 'sdc.skilleddesk.com',
    private readonly origin = 'https://sdc.skilleddesk.com',
    /** Bit 0 user present, bit 2 user verified. Set to 0x01 to model a passkey that skipped verification. */
    public flags = 0x05,
  ) {}

  async create(_userName?: string, userHandle?: Bytes): Promise<{ id: string; publicKey: Bytes }> {
    const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])) as CryptoKeyPair;
    const id = b64u(crypto.getRandomValues(new Uint8Array(16)));
    const prfSecret = await crypto.subtle.importKey('raw', crypto.getRandomValues(new Uint8Array(32)), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);

    this.credentials.set(id, { privateKey: pair.privateKey, counter: 0, userHandle: userHandle ?? crypto.getRandomValues(new Uint8Array(16)), prfSecret });

    return { id, publicKey: new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)) };
  }

  async assert(id: string, challenge: Bytes): Promise<AssertionWire> {
    const credential = this.credentials.get(id);

    if (!credential) throw new Error('unknown credential');

    credential.counter += 1;

    const clientData = utf8(JSON.stringify({ type: 'webauthn.get', challenge: b64u(challenge), origin: this.origin, crossOrigin: false }));
    const counter = new Uint8Array(4);

    new DataView(counter.buffer).setUint32(0, credential.counter);

    const authenticatorData = concat(await sha256(utf8(this.rpId)), Uint8Array.from([this.flags]), counter);
    const signed = concat(authenticatorData, await sha256(clientData));
    const raw = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, credential.privateKey, signed));

    return { authenticator_data: b64u(authenticatorData), client_data_json: b64u(clientData), signature: b64u(derFromRaw(raw)) };
  }

  private async prfOf(credential: Credential, salt: Bytes): Promise<Bytes | null> {
    // WebAuthn hashes the salt with a context string before it reaches the authenticator; any fixed mapping models that.
    return this.prf ? (new Uint8Array(await crypto.subtle.sign('HMAC', credential.prfSecret, concat(utf8('WebAuthn PRF\0'), salt))) as Bytes) : null;
  }

  async assertWithPrf(id: string, challenge: Bytes, salt: Bytes): Promise<{ wire: AssertionWire; prf: Bytes | null }> {
    const wire = await this.assert(id, challenge);
    const credential = this.credentials.get(id)!;

    return { wire, prf: await this.prfOf(credential, salt) };
  }

  /** The person picks "the" passkey: here, the most recently made one that has a user handle. */
  async recover(salt: Bytes): Promise<{ id: string; userHandle: Bytes; prf: Bytes | null } | null> {
    const entries = [...this.credentials.entries()];
    const last = entries[entries.length - 1];

    if (!last) return null;

    return { id: last[0], userHandle: last[1].userHandle, prf: await this.prfOf(last[1], salt) };
  }
}

export { rawKeyFromSpki, fromB64u };
