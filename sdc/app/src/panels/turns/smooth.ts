import { useEffect, useRef, useState } from 'react';

/**
 * Streamed text, revealed evenly (0.11.8).
 *
 * The daemon pushes what an engine said every 50 ms, so the raw text grows in chunks of ten to forty
 * characters - a stutter the eye reads as lag. This hook draws the text a few characters per frame
 * instead, always a fixed fraction of what is still behind, so it flows at the engine's own pace, never
 * falls more than a few frames behind, and settles the moment the stream does.
 *
 * Two details keep it honest:
 *
 *   * it never stops inside a character: a surrogate pair, a combining mark, a zero-width joiner, or a
 *     Bengali conjunct after a hasant (্) is taken whole, so "ক্ষ" never flashes as "ক্" first;
 *   * a turn that is not streaming (a replay, a finished answer) is drawn in full at once, and so is
 *     everything for a person who asked the system for reduced motion.
 */
export function useSmoothText(target: string, streaming: boolean): string {
  const [shown, setShown] = useState(() => (streaming ? '' : target));
  const current = useRef(shown);

  useEffect(() => {
    if (!streaming || prefersReducedMotion()) {
      current.current = target;
      setShown(target);
      return;
    }

    let frame = 0;

    const step = (): void => {
      const now = current.current;

      /* The text was replaced rather than extended (a retry, a rewind): draw what is true. */
      if (!target.startsWith(now)) {
        current.current = target;
        setShown(target);
        return;
      }

      const behind = target.length - now.length;

      if (behind <= 0) {
        return;
      }

      let end = now.length + Math.max(1, Math.ceil(behind / 6));

      while (end < target.length && continues(target, end)) {
        end += 1;
      }

      const next = target.slice(0, end);

      current.current = next;
      setShown(next);

      if (end < target.length) {
        frame = globalThis.requestAnimationFrame(step);
      }
    };

    frame = globalThis.requestAnimationFrame(step);

    return () => globalThis.cancelAnimationFrame(frame);
  }, [target, streaming]);

  return streaming ? shown : target;
}

const MARK = /\p{M}/u;

/** Does the character at `index` belong to the one before it? */
export function continues(text: string, index: number): boolean {
  const code = text.charCodeAt(index);

  /* The second half of a surrogate pair, a zero-width (non-)joiner, a variation selector. */
  if ((code >= 0xdc00 && code <= 0xdfff) || code === 0x200c || code === 0x200d || (code >= 0xfe00 && code <= 0xfe0f)) {
    return true;
  }

  /* A combining mark: Bengali vowel signs, nasalisation, diacritics. */
  if (MARK.test(text[index] ?? '')) {
    return true;
  }

  /* After a hasant/virama the next consonant is part of the same conjunct. */
  const before = text.charCodeAt(index - 1);

  return before === 0x09cd || before === 0x094d;
}

function prefersReducedMotion(): boolean {
  return typeof globalThis.matchMedia === 'function' && globalThis.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/**
 * The `Understood: …` line the reading brief asks an engine to start with (daemon: `understand.rs`),
 * split from the rest of the answer so the window can draw it as its own card.
 *
 * Models dress the marker up (`**Understood:**`, `` `Understood:` ``), so the match allows for that. While
 * the first line is still arriving, a text that is only the beginning of the marker (`Unders`) is held
 * back rather than drawn as the answer's first word.
 */
export function splitUnderstood(text: string, streaming: boolean): { understood: string | null; rest: string } {
  const trimmed = text.replace(/^\s+/, '');
  const marker = /^(?:\*\*|__|`)?Understood:(?:\*\*|__|`)?[ \t]*/i;
  const found = marker.exec(trimmed);

  if (found === null) {
    if (streaming && trimmed.length < 16 && /^(?:\*\*|__|`)?u?n?d?e?r?s?t?o?o?d?$/i.test(trimmed) && trimmed !== '') {
      return { understood: null, rest: '' };
    }

    return { understood: null, rest: text };
  }

  const body = trimmed.slice(found[0].length);
  const newline = body.indexOf('\n');

  if (newline === -1) {
    return { understood: body.trim(), rest: '' };
  }

  return { understood: body.slice(0, newline).trim(), rest: body.slice(newline + 1).replace(/^\s*\n/, '') };
}
