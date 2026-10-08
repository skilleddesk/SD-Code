// Passkeys (WebAuthn), behind an interface so the app uses the browser's and the tests use a software one.
//
// Every assertion this app asks for has user verification required: the passkey must check a fingerprint,
// a face or a PIN. The daemon verifies the assertion itself (sdcd/src/anywhere/webauthn.rs); the relay's
// opinion is not asked.

import { b64u, fromB64u, type Bytes } from './bytes';

export interface AssertionWire {
  authenticator_data: string;
  client_data_json: string;
  signature: string;
}

export interface Passkey {
  /**
   * Makes a new passkey for this site. Returns its id and its public key (SEC1, 65 bytes). `userHandle` is stored in the passkey
   * and handed back by `recover`; the app puts the computer's id in it so a browser that forgot everything can still find its way.
   */
  create(userName: string, userHandle?: Bytes): Promise<{ id: string; publicKey: Bytes }>;
  /** Signs `challenge` (raw bytes) with passkey `id`, with user verification. */
  assert(id: string, challenge: Bytes): Promise<AssertionWire>;
  /** Like `assert`, and also asks the passkey for its PRF output over `salt` (null when this passkey has none). */
  assertWithPrf?(id: string, challenge: Bytes, salt: Bytes): Promise<{ wire: AssertionWire; prf: Bytes | null }>;
  /**
   * Lets the person pick one of this site's passkeys without the app knowing which: for rejoining after the browser's own data was
   * cleared. Returns the passkey's id, the user handle it was made with and its PRF output (null when it has none). Null if cancelled.
   */
  recover?(salt: Bytes): Promise<{ id: string; userHandle: Bytes; prf: Bytes | null } | null>;
}

/** The 65-byte uncompressed point at the end of a P-256 SubjectPublicKeyInfo. */
export function rawKeyFromSpki(spki: Bytes): Bytes {
  if (spki.length < 65 || spki[spki.length - 65] !== 4) throw new Error('not a P-256 public key');
  return spki.slice(spki.length - 65);
}

const buffer = (bytes: Bytes): ArrayBuffer => bytes.slice().buffer as ArrayBuffer;

/** The `prf` result of a get(), if the authenticator produced one. */
function prfOf(credential: PublicKeyCredential): Bytes | null {
  const results = (credential.getClientExtensionResults() as { prf?: { results?: { first?: ArrayBuffer } } }).prf?.results?.first;

  return results ? (new Uint8Array(results) as Bytes) : null;
}

function wireOf(credential: PublicKeyCredential): AssertionWire {
  const response = credential.response as AuthenticatorAssertionResponse;

  return {
    authenticator_data: b64u(new Uint8Array(response.authenticatorData)),
    client_data_json: b64u(new Uint8Array(response.clientDataJSON)),
    signature: b64u(new Uint8Array(response.signature)),
  };
}

export class BrowserPasskey implements Passkey {
  constructor(private readonly rpId: string = location.hostname) {}

  async create(userName: string, userHandle?: Bytes): Promise<{ id: string; publicKey: Bytes }> {
    const credential = (await navigator.credentials.create({
      publicKey: {
        rp: { id: this.rpId, name: 'SDC Anywhere' },
        user: { id: buffer(userHandle ?? (crypto.getRandomValues(new Uint8Array(16)) as Bytes)), name: userName, displayName: userName },
        challenge: crypto.getRandomValues(new Uint8Array(32)),
        pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
        // Discoverable, so the passkey can be found again without knowing its id; `prf` so it can seal the rejoin vault.
        authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
        extensions: { prf: {} } as AuthenticationExtensionsClientInputs,
        attestation: 'none',
        timeout: 120_000,
      },
    })) as PublicKeyCredential | null;

    if (!credential) throw new Error('no passkey was created');

    const response = credential.response as AuthenticatorAttestationResponse;
    const spki = response.getPublicKey();

    if (!spki) throw new Error('this browser did not give the passkey public key');

    return { id: b64u(new Uint8Array(credential.rawId)), publicKey: rawKeyFromSpki(new Uint8Array(spki)) };
  }

  async assert(id: string, challenge: Bytes): Promise<AssertionWire> {
    return (await this.assertWithPrf(id, challenge, null)).wire;
  }

  async assertWithPrf(id: string, challenge: Bytes, salt: Bytes | null): Promise<{ wire: AssertionWire; prf: Bytes | null }> {
    const credential = (await navigator.credentials.get({
      publicKey: {
        rpId: this.rpId,
        challenge: buffer(challenge),
        allowCredentials: [{ type: 'public-key', id: buffer(fromB64u(id)) }],
        userVerification: 'required',
        ...(salt ? { extensions: { prf: { eval: { first: buffer(salt) } } } as AuthenticationExtensionsClientInputs } : {}),
        timeout: 120_000,
      },
    })) as PublicKeyCredential | null;

    if (!credential) throw new Error('the passkey was not used');

    return { wire: wireOf(credential), prf: salt ? prfOf(credential) : null };
  }

  async recover(salt: Bytes): Promise<{ id: string; userHandle: Bytes; prf: Bytes | null } | null> {
    const credential = (await navigator.credentials.get({
      publicKey: {
        rpId: this.rpId,
        challenge: crypto.getRandomValues(new Uint8Array(32)),
        userVerification: 'required',
        extensions: { prf: { eval: { first: buffer(salt) } } } as AuthenticationExtensionsClientInputs,
        timeout: 120_000,
      },
    })) as PublicKeyCredential | null;

    if (!credential) return null;

    const handle = (credential.response as AuthenticatorAssertionResponse).userHandle;

    if (!handle) throw new Error('this passkey does not say which computer it belongs to');

    return { id: b64u(new Uint8Array(credential.rawId)), userHandle: new Uint8Array(handle) as Bytes, prf: prfOf(credential) };
  }
}

/** WebAuthn gives the signature as DER; a software authenticator needs to produce it from WebCrypto's raw r||s. */
export function derFromRaw(raw: Bytes): Bytes {
  const integer = (bytes: Bytes): Bytes => {
    let start = 0;
    while (start < bytes.length - 1 && bytes[start] === 0) start++;
    let body = bytes.slice(start);
    if ((body[0] ?? 0) & 0x80) body = Uint8Array.from([0, ...body]);
    return Uint8Array.from([0x02, body.length, ...body]);
  };
  const r = integer(raw.slice(0, 32));
  const s = integer(raw.slice(32, 64));

  return Uint8Array.from([0x30, r.length + s.length, ...r, ...s]);
}
