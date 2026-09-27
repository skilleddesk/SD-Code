import { describe, expect, it } from 'vitest';

import { paragraphs } from '../../lib/markdown';
import { continues, splitUnderstood } from './smooth';

describe('the Understood line (0.11.8)', () => {
  it('is split from the answer, however the model dresses the marker', () => {
    for (const text of [
      'Understood: সাইটটা কেন ধীর তা খুঁজে ঠিক করতে হবে।\n\nপ্রথমে…',
      '**Understood:** সাইটটা কেন ধীর তা খুঁজে ঠিক করতে হবে।\n\nপ্রথমে…',
      '  `Understood:` সাইটটা কেন ধীর তা খুঁজে ঠিক করতে হবে।\nপ্রথমে…',
    ]) {
      const { understood, rest } = splitUnderstood(text, false);

      expect(understood).toBe('সাইটটা কেন ধীর তা খুঁজে ঠিক করতে হবে।');
      expect(rest).toBe('প্রথমে…');
    }
  });

  it('grows as its line streams in, and holds back a half-typed marker', () => {
    expect(splitUnderstood('Unders', true)).toEqual({ understood: null, rest: '' });
    expect(splitUnderstood('Understood: সাইট', true)).toEqual({ understood: 'সাইট', rest: '' });
  });

  it('leaves an answer without the marker alone', () => {
    expect(splitUnderstood('Done - 3 files changed.', false)).toEqual({ understood: null, rest: 'Done - 3 files changed.' });
    expect(splitUnderstood('Under the hood, it works.', true).rest).toBe('Under the hood, it works.');
  });
});

describe('the smooth reveal never stops inside a character', () => {
  it('keeps a Bengali conjunct and vowel signs whole', () => {
    const word = 'ক্ষমা';

    /* After ক comes the hasant (a mark), after the hasant comes ষ (joined), and া is a mark. */
    expect(continues(word, 1)).toBe(true);
    expect(continues(word, 2)).toBe(true);
    expect(continues(word, 3)).toBe(false);
    expect(continues(word, 4)).toBe(true);
  });

  it('keeps a surrogate pair whole', () => {
    expect(continues('a😀', 2)).toBe(true);
    expect(continues('ab', 1)).toBe(false);
  });
});

describe('paragraphs', () => {
  it('splits at blank lines but never inside a code fence', () => {
    expect(paragraphs('one\ntwo\n\nthree\n\n```\na\n\nb\n```\n\nfour')).toEqual(['one\ntwo', 'three', '```\na\n\nb\n```', 'four']);
  });
});
