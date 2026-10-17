/**
 * One token estimate for the UI (Phase 0.9) - the same algorithm as the daemon's `sdcd/src/tokens.rs`.
 *
 * The turn bar used to show `chars / 4`, which is right for English and about 4x too low for Bengali. This is a
 * script-aware, upper-biased estimate calibrated against real tokenizers; `protocol/token-fixtures.json` holds the real
 * counts and the number this function must return, and both the Rust and the TypeScript tests read it, so the two cannot
 * drift apart. Change them together.
 */

const ARABIC = 0.9;
const BRAHMIC = 1.5;
const CJK = 1.1;
const CYRILLIC = 0.55;
const GREEK = 1.0;
const HEBREW = 1.15;
const EMOJI = 3.0;
const OTHER = 2.0;
const PUNCT = 0.7;
const NEWLINE = 0.5;
const WORD = 5.0;
const MARGIN = 1.08;

type Kind = 'space' | 'newline' | 'word' | 'punct' | number;

function kindOf(char: string): Kind {
  const code = char.codePointAt(0) ?? 0;

  if (char === '\n') return 'newline';
  if (/\s/u.test(char)) return 'space';
  if (code < 128) return /[A-Za-z0-9_]/u.test(char) ? 'word' : 'punct';
  if ((code >= 0x0600 && code <= 0x06ff) || (code >= 0x0750 && code <= 0x077f) || (code >= 0xfb50 && code <= 0xfdff) || (code >= 0xfe70 && code <= 0xfeff)) return ARABIC;
  if ((code >= 0x0900 && code <= 0x0dff) || (code >= 0x0e00 && code <= 0x0eff) || (code >= 0x1000 && code <= 0x109f) || (code >= 0x1780 && code <= 0x17ff)) return BRAHMIC;
  if ((code >= 0x3040 && code <= 0x30ff) || (code >= 0x3400 && code <= 0x9fff) || (code >= 0xac00 && code <= 0xd7af) || (code >= 0xf900 && code <= 0xfaff)) return CJK;
  if ((code >= 0x00c0 && code <= 0x024f) || (code >= 0x1e00 && code <= 0x1eff)) return 'word';
  if (code >= 0x0400 && code <= 0x052f) return CYRILLIC;
  if (code >= 0x0370 && code <= 0x03ff) return GREEK;
  if (code >= 0x0590 && code <= 0x05ff) return HEBREW;
  if (code >= 0x1f000 || (code >= 0x2600 && code <= 0x27bf)) return EMOJI;

  return OTHER;
}

/** The estimated tokens of `text` - never fewer than a real tokenizer counts. */
export function estimateTokens(text: string): number {
  if (text === '') return 0;

  let total = 0;
  let run = 0;

  for (const char of text) {
    const kind = kindOf(char);

    if (kind === 'word') {
      run += 1;
      continue;
    }

    if (run > 0) {
      total += Math.ceil(run / WORD);
      run = 0;
    }

    if (kind === 'newline') total += NEWLINE;
    else if (kind === 'punct') total += PUNCT;
    else if (typeof kind === 'number') total += kind;
  }

  if (run > 0) total += Math.ceil(run / WORD);

  return Math.ceil(total * MARGIN) + 1;
}

/** For a length with no text to look at (a file being written): ASCII-sized, a third of it - upper-biased. */
export function estimateTokensOfChars(chars: number): number {
  return Math.ceil(chars / 3);
}
