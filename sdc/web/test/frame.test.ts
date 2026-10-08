import { describe, expect, it } from 'vitest';
import { FRAGMENT, MAX_MESSAGE, MAX_PARTIAL, Reassembler, pieces, whole } from '../src/crypto/frame';

const bytes = (length: number, fill = 7): Uint8Array<ArrayBuffer> => new Uint8Array(length).fill(fill);

describe('fragments', () => {
  it('a small message is one whole frame', () => {
    const [only, ...rest] = pieces(bytes(10), 1);

    expect(rest).toHaveLength(0);
    expect(only![0]).toBe(0);
    expect(new Reassembler().push(only!)).toEqual(bytes(10));
  });

  it('a big message comes back whole after its pieces', () => {
    const message = Uint8Array.from({ length: FRAGMENT * 3 + 123 }, (_, n) => n % 251);
    const all = pieces(message, 9);
    const rx = new Reassembler();
    let done: Uint8Array | null = null;

    expect(all).toHaveLength(4);
    expect(all.every((piece) => piece.length <= FRAGMENT + 5)).toBe(true);

    for (const piece of all) done = rx.push(piece);

    expect(done).toEqual(message);
  });

  it('an exactly full frame is still whole', () => {
    expect(pieces(bytes(FRAGMENT), 1)).toHaveLength(1);
    expect(pieces(bytes(FRAGMENT + 1), 1)).toHaveLength(2);
  });

  it('pieces of two messages can be interleaved', () => {
    const [a0, a1, a2] = pieces(bytes(FRAGMENT * 2 + 1, 1), 1);
    const [b0, b1] = pieces(bytes(FRAGMENT + 1, 2), 2);
    const rx = new Reassembler();
    const finished = [a0, b0, a1, b1, a2].map((piece) => rx.push(piece!)).filter((message) => message !== null);

    expect(finished).toHaveLength(2);
    expect(finished[0]![0]).toBe(2);
    expect(finished[1]![0]).toBe(1);
  });

  it('refuses what it could not hold, and garbage', () => {
    const rx = new Reassembler();

    for (let id = 0; id < MAX_PARTIAL; id++) rx.push(Uint8Array.from([1, 0, 0, 0, id, 1]));

    expect(() => rx.push(Uint8Array.from([1, 0, 0, 1, 0, 1]))).toThrow(/too many/);
    expect(() => new Reassembler().push(new Uint8Array(0))).toThrow();
    expect(() => new Reassembler().push(Uint8Array.from([9, 1]))).toThrow(/unknown/);
    expect(() => new Reassembler().push(Uint8Array.from([1, 0]))).toThrow(/shorter/);

    const big = new Reassembler();
    const piece = new Uint8Array(5 + FRAGMENT);

    piece[0] = 1;
    piece[4] = 9;

    expect(() => {
      for (let i = 0; i < MAX_MESSAGE / FRAGMENT + 2; i++) big.push(piece);
    }).toThrow(/size limit/);
  });

  it('matches the daemon\'s wire form for a whole frame', () => {
    expect([...whole(Uint8Array.from([120]))]).toEqual([0, 120]);
  });
});
