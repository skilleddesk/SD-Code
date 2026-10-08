// Fragments (mirror of sdcd/src/anywhere/frame.rs): how a message too big for one frame travels.
//
// Every sealed plaintext starts with a flag byte: 0 = the whole message; 1 = a piece, more follow; 2 = the last piece.
// A piece also carries a 4-byte message id, so pieces of different messages can be interleaved and the sender can put
// an urgent message between the pieces of a big one. Pieces of one message arrive in order (the session numbers every
// frame), so putting them back together is concatenation. A receiver bounds what it will hold.

import { concat, type Bytes } from './bytes';

export const FRAGMENT = 64 * 1024;
export const MAX_MESSAGE = 8 * 1024 * 1024;
export const MAX_PARTIAL = 8;

const WHOLE = 0;
const MORE = 1;
const LAST = 2;

export function whole(bytes: Bytes): Bytes {
  return concat(Uint8Array.from([WHOLE]), bytes);
}

/** The plaintexts to seal, in order, for one message. */
export function pieces(bytes: Bytes, id: number): Bytes[] {
  if (bytes.length <= FRAGMENT) return [whole(bytes)];

  const out: Bytes[] = [];

  for (let offset = 0; offset < bytes.length; offset += FRAGMENT) {
    const end = Math.min(offset + FRAGMENT, bytes.length);
    const header = new Uint8Array(5);

    header[0] = end === bytes.length ? LAST : MORE;
    new DataView(header.buffer).setUint32(1, id >>> 0);
    out.push(concat(header, bytes.slice(offset, end)));
  }

  return out;
}

export class Reassembler {
  private partial = new Map<number, Bytes[]>();
  private sizes = new Map<number, number>();

  /** Takes one opened frame. Returns the message when it completed one; throws on anything a sender must not do. */
  push(plain: Bytes): Bytes | null {
    const kind = plain[0];

    if (kind === undefined) throw new Error('an empty frame');

    if (kind === WHOLE) {
      if (plain.length - 1 > MAX_MESSAGE) throw new Error('a message above the size limit');

      return plain.slice(1);
    }

    if (kind !== MORE && kind !== LAST) throw new Error(`unknown frame kind ${kind}`);
    if (plain.length < 5) throw new Error('a piece shorter than its header');

    const id = new DataView(plain.buffer, plain.byteOffset).getUint32(1);
    const piece = plain.slice(5);

    if (!this.partial.has(id) && this.partial.size >= MAX_PARTIAL) throw new Error('too many unfinished messages');

    const size = (this.sizes.get(id) ?? 0) + piece.length;

    if (size > MAX_MESSAGE) {
      this.partial.delete(id);
      this.sizes.delete(id);

      throw new Error('a message above the size limit');
    }

    this.partial.set(id, [...(this.partial.get(id) ?? []), piece]);
    this.sizes.set(id, size);

    if (kind === LAST) {
      const message = concat(...(this.partial.get(id) ?? []));

      this.partial.delete(id);
      this.sizes.delete(id);

      return message;
    }

    return null;
  }
}
