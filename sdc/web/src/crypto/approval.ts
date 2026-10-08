// What a card is, what must back an answer to it, and what the browser signs.

import { b64u, fromB64u, utf8, type Bytes } from './bytes';
import { canonical, type Json } from './canonical';
import { sha256 } from './bytes';

/** The envelope the daemon shows (sdcd `ActionEnvelope::to_value`). */
export interface Envelope {
  v: number;
  request_id: string;
  turn_id: string;
  session_id: string;
  host: string;
  cwd: string;
  action: string;
  target: string;
  args: string[];
  file_hashes: Record<string, string>;
  risk: string;
  blast_radius: { files: number; db_tables: number; services: string[]; notes: string[] };
  title: string;
  reason: string;
  rollback: string;
  est_cost_micro_usd: number;
  expires_at: number;
  nonce: string;
}

export interface Card {
  envelope: Envelope;
  /** base64url SHA-256 of the canonical envelope, as the daemon computed it. */
  action_hash: string;
}

/** Same rule as the daemon (`router::needs_fresh_passkey`). The daemon enforces it; the browser only knows what to ask for. */
export function needsFreshPasskey(envelope: Pick<Envelope, 'risk' | 'action'>): boolean {
  return envelope.risk === 'DANGEROUS' || ['delete', 'deploy', 'rewind'].includes(envelope.action);
}

/**
 * Whether the card's hash really is the hash of the card. The browser recomputes it before it signs, so a
 * relay that swapped the envelope but kept the old hash (or the reverse) is caught on the phone, not just
 * on the computer.
 */
export async function hashMatches(card: Card): Promise<boolean> {
  const bytes = await sha256(utf8(canonical(card.envelope as unknown as Json)));

  return b64u(bytes) === card.action_hash;
}

export function hashBytes(card: Card): Bytes {
  return fromB64u(card.action_hash);
}

/** A readable expiry for the card. */
export function secondsLeft(envelope: Envelope, now: number): number {
  return Math.max(0, Math.round((envelope.expires_at - now) / 1000));
}
