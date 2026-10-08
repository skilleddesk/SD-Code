// The browser's half of the end-to-end session (mirror of sdcd/src/anywhere/crypto.rs).
//
// Two directions, two HPKE contexts (RFC 9180 base mode, DHKEM(X25519, HKDF-SHA256), HKDF-SHA256,
// AES-256-GCM). An HPKE context numbers its own messages, so a dropped, repeated or reordered frame fails
// to open: the relay cannot replay or reorder anything without the session noticing.
//
//   device                                                     daemon
//     | hello { device, enc, ct, sig }  --------------------->  |  ct = Seal_c2d(HelloPlain)
//     |        sig = ECDSA(device key, label | enc | ct)          |
//     | <-----------------  welcome { enc, ct, sig }              |  ct = Seal_d2c(WelcomePlain)
//     |        sig = ECDSA(daemon key, label | hello.sig | enc | ct)
//
// The device key is a WebCrypto non-extractable ECDSA P-256 key. The daemon's identity key and KEM key were
// pinned from the pairing QR code; a welcome that does not verify against the pinned key is refused.

import { AeadId, Aes256Gcm, CipherSuite, HkdfSha256, KdfId, KemId } from '@hpke/core';
import { DhkemX25519HkdfSha256 } from '@hpke/dhkem-x25519';
import { b64u, concat, counterBytes, fromB64u, random, sha256, utf8, type Bytes } from './bytes';

const LABEL_HELLO = utf8('sdc-anywhere/v1/hello');
const LABEL_WELCOME = utf8('sdc-anywhere/v1/welcome');
export const LABEL_DECISION = utf8('sdc-anywhere/v1/decision');
const INFO_C2D = 'sdc-anywhere/v1/c2d';
const INFO_D2C = 'sdc-anywhere/v1/d2c';
const LABEL_PAIR_CHALLENGE = 'sdc-anywhere/v1/pair-challenge';
const LABEL_SAS = 'sdc-anywhere/v1/sas';
const LABEL_UNLOCK = utf8('sdc-anywhere/v1/unlock');

function suite(): CipherSuite {
  return new CipherSuite({ kem: new DhkemX25519HkdfSha256(), kdf: new HkdfSha256(), aead: new Aes256Gcm() });
}

// Referenced so a bundler keeps the ids: they document which ciphersuite this is.
export const CIPHERSUITE = { kem: KemId.DhkemX25519HkdfSha256, kdf: KdfId.HkdfSha256, aead: AeadId.Aes256Gcm } as const;

/** What a device remembers about the daemon it paired with. */
export interface DaemonPin {
  id: string;
  /** SEC1 ECDSA P-256 public key. */
  identityPublic: Bytes;
  /** X25519 KEM public key. */
  kemPublic: Bytes;
}

export interface Hello {
  t: 'hello';
  device: string;
  enc: string;
  ct: string;
  sig: string;
}

export interface Welcome {
  t: 'welcome';
  enc: string;
  ct: string;
  sig: string;
}

export interface PairBody {
  token: string;
  name: string;
  user_agent: string;
  sign_pub: string;
  passkey_id: string;
  passkey_pub: string;
  assertion: { authenticator_data: string; client_data_json: string; signature: string } | null;
}

export interface WelcomePlain {
  device: string;
  ts: number;
  nonce: string;
  level: string;
  seq: number;
}

/** The id a daemon is known by: first 16 bytes of SHA-256 of its identity public key. */
export async function daemonIdOf(identityPublic: Bytes): Promise<string> {
  return b64u((await sha256(identityPublic)).slice(0, 16));
}

/** Ten bytes of the fingerprint in groups, the same form the desktop shows. */
export async function fingerprint(identityPublic: Bytes): Promise<string> {
  const digest = await sha256(identityPublic);
  const groups: string[] = [];

  for (let i = 0; i < 10; i += 2) groups.push(`${hex(digest[i])}${hex(digest[i + 1])}`);

  return groups.join('-');
}

function hex(byte: number | undefined): string {
  return (byte ?? 0).toString(16).padStart(2, '0');
}

export async function signWith(key: CryptoKey, message: Bytes): Promise<Bytes> {
  return new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, message));
}

export async function verifyWith(publicKey: Bytes, message: Bytes, signature: Bytes): Promise<boolean> {
  try {
    const key = await crypto.subtle.importKey('raw', publicKey, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);

    return await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, signature, message);
  } catch {
    return false;
  }
}

/** The challenge a new device's passkey signs: bound to its own hello. */
export async function pairChallenge(nonce: string): Promise<Bytes> {
  return sha256(utf8(LABEL_PAIR_CHALLENGE), utf8(nonce));
}

/** The six digits both screens show during pairing (`123 456`). */
export async function sasCode(token: string, deviceSignPublic: Bytes, daemonIdentityPublic: Bytes): Promise<string> {
  const digest = await sha256(utf8(LABEL_SAS), utf8(token), deviceSignPublic, daemonIdentityPublic);
  const number = new DataView(digest.buffer, digest.byteOffset).getUint32(0) % 1_000_000;

  return `${String(Math.floor(number / 1000)).padStart(3, '0')} ${String(number % 1000).padStart(3, '0')}`;
}

/** The challenge that opens a capability level: bound to the level and the daemon's nonce. */
export async function unlockChallenge(level: string, nonce: Bytes): Promise<Bytes> {
  return sha256(LABEL_UNLOCK, utf8(level), nonce);
}

/** What a device signs for a decision; the daemon checks it against the action hash it holds. */
export function decisionMessage(hash: Bytes, decision: string): Bytes {
  return concat(LABEL_DECISION, hash, utf8(decision));
}

export class SendHalf {
  private count = 0;

  constructor(private readonly ctx: { seal(data: ArrayBuffer, aad?: ArrayBuffer): Promise<ArrayBuffer> }) {}

  get sent(): number {
    return this.count;
  }

  async seal(plain: Bytes): Promise<{ n: number; ct: Bytes }> {
    const n = this.count;
    const ct = new Uint8Array(await this.ctx.seal(plain.slice().buffer, counterBytes(n).buffer as ArrayBuffer));

    this.count += 1;

    return { n, ct };
  }
}

export class RecvHalf {
  private count = 0;

  constructor(private readonly ctx: { open(data: ArrayBuffer, aad?: ArrayBuffer): Promise<ArrayBuffer> }) {}

  get received(): number {
    return this.count;
  }

  /** Opens message `n`, which must be exactly the next one. Throws if the frame was dropped, repeated or altered. */
  async open(n: number, ct: Bytes): Promise<Bytes> {
    if (n !== this.count) throw new Error('frame out of order');

    const plain = new Uint8Array(await this.ctx.open(ct.slice().buffer, counterBytes(n).buffer as ArrayBuffer));

    this.count += 1;

    return plain;
  }
}

export interface Established {
  send: SendHalf;
  recv: RecvHalf;
  welcome: WelcomePlain;
}

/** A handshake in progress: the `hello` to send and the function that finishes it with the `welcome`. */
export interface Handshake {
  hello: Hello;
  /** The hello's nonce, which a pairing assertion must be bound to. */
  nonce: string;
  finish(welcome: Welcome): Promise<Established>;
}

export interface StartOptions {
  signKey: CryptoKey;
  deviceId: string;
  daemon: DaemonPin;
  lastSeq: number;
  now: number;
  /** For a brand-new device: builds the pairing body for this hello's nonce (it needs the nonce for its passkey proof). */
  pair?: (nonce: string) => Promise<PairBody>;
}

export async function startHandshake(options: StartOptions): Promise<Handshake> {
  const { signKey, deviceId, daemon } = options;
  const cipher = suite();
  const recipientPublicKey = await cipher.kem.deserializePublicKey(daemon.kemPublic.slice().buffer as ArrayBuffer);
  const sessionKeys = await cipher.kem.generateKeyPair();
  const sessionPublic = new Uint8Array(await cipher.kem.serializePublicKey(sessionKeys.publicKey));
  const nonce = b64u(random(16));
  const plain: Record<string, unknown> = { device: deviceId, pk: b64u(sessionPublic), ts: options.now, nonce, last_seq: options.lastSeq };

  if (options.pair) plain.pair = await options.pair(nonce);

  const sender = await cipher.createSenderContext({ recipientPublicKey, info: utf8(`${INFO_C2D}|${daemon.id}|${deviceId}`).slice().buffer as ArrayBuffer });
  const ct = new Uint8Array(await sender.seal(utf8(JSON.stringify(plain)).slice().buffer as ArrayBuffer, new ArrayBuffer(0)));
  const enc = new Uint8Array(sender.enc);
  const sig = await signWith(signKey, concat(LABEL_HELLO, enc, ct));
  const hello: Hello = { t: 'hello', device: deviceId, enc: b64u(enc), ct: b64u(ct), sig: b64u(sig) };

  return {
    hello,
    nonce,
    async finish(welcome: Welcome): Promise<Established> {
      const wEnc = fromB64u(welcome.enc);
      const wCt = fromB64u(welcome.ct);
      const wSig = fromB64u(welcome.sig);

      if (!(await verifyWith(daemon.identityPublic, concat(LABEL_WELCOME, sig, wEnc, wCt), wSig))) {
        throw new Error('the welcome is not signed by the computer this device paired with');
      }

      const recipient = await cipher.createRecipientContext({
        recipientKey: sessionKeys.privateKey,
        enc: wEnc.slice().buffer as ArrayBuffer,
        info: utf8(`${INFO_D2C}|${daemon.id}|${deviceId}`).slice().buffer as ArrayBuffer,
      });
      const body = JSON.parse(new TextDecoder().decode(new Uint8Array(await recipient.open(wCt.slice().buffer as ArrayBuffer, new ArrayBuffer(0))))) as WelcomePlain;

      if (body.nonce !== nonce || body.device !== deviceId) throw new Error('the welcome answers a different hello');

      return { send: new SendHalf(sender), recv: new RecvHalf(recipient), welcome: body };
    },
  };
}
