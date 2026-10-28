import { describe, expect, it } from 'vitest';

import { heldLanguage, MAX_PHRASE_MS, PhraseCutter, tidyDictation, type Phrase } from './dictation';

/**
 * A recorded-like signal: words of a voice (a tone whose loudness rises and falls like syllables) separated by
 * the gaps people leave between words and the pauses between sentences, over a constant room noise.
 */
const RATE = 48_000;
const CHUNK = 4096;

/** A seeded random, so every run hears the same "recording". */
function random(seed: number): () => number {
  let state = seed;

  return () => {
    state = (state * 1_664_525 + 1_013_904_223) % 4_294_967_296;

    return state / 4_294_967_296;
  };
}

interface Script {
  samples: Float32Array;
  /** Per sample: is it speech? */
  speech: Uint8Array;
}

/** `[kind, ms, loudness]`: `w` a word, `g` a gap or pause. */
function record(parts: readonly ['w' | 'g', number, number?][], noise = 0.003, seed = 7): Script {
  const total = parts.reduce((sum, [, ms]) => sum + Math.round((ms / 1000) * RATE), 0);
  const samples = new Float32Array(total);
  const speech = new Uint8Array(total);
  const next = random(seed);
  let at = 0;

  for (const [kind, ms, loudness = 0.06] of parts) {
    const length = Math.round((ms / 1000) * RATE);

    for (let index = 0; index < length; index += 1) {
      const t = index / RATE;
      const room = (next() * 2 - 1) * noise * 1.7;

      if (kind === 'w') {
        /* Syllables: about five a second, never fully silent inside a word. */
        const envelope = 0.55 + 0.45 * Math.sin(2 * Math.PI * 5 * t - Math.PI / 2);
        const voice = Math.sin(2 * Math.PI * 180 * t) * 0.7 + Math.sin(2 * Math.PI * 360 * t) * 0.3;

        samples[at + index] = voice * loudness * 1.41 * envelope + room;
        speech[at + index] = 1;
      } else {
        samples[at + index] = room;
      }
    }

    at += length;
  }

  return { samples, speech };
}

/** Feeds a recording through the cutter chunk by chunk, as the microphone would, and presses stop at the end. */
function listen(script: Script): { sent: Phrase[]; offsets: Map<Float32Array, number> } {
  const cutter = new PhraseCutter(RATE);
  const sent: Phrase[] = [];
  const offsets = new Map<Float32Array, number>();

  for (let at = 0; at + CHUNK <= script.samples.length; at += CHUNK) {
    const chunk = script.samples.slice(at, at + CHUNK);

    offsets.set(chunk, at);

    const { ended } = cutter.push(chunk, ((at + CHUNK) / RATE) * 1000);

    if (ended !== null) {
      sent.push(ended);
    }
  }

  const last = cutter.flush();

  if (last !== null) {
    sent.push(last);
  }

  return { sent, offsets };
}

/** How much of the speech reached a phrase that was sent to the transcriber (0..1). */
function coverage(script: Script, sent: readonly Phrase[], offsets: Map<Float32Array, number>): number {
  const heard = new Uint8Array(script.samples.length);

  for (const phrase of sent) {
    for (const chunk of phrase.chunks) {
      heard.fill(1, offsets.get(chunk) ?? 0, (offsets.get(chunk) ?? 0) + chunk.length);
    }
  }

  let spoken = 0;
  let caught = 0;
  /* The last partial chunk is never delivered by a microphone either. */
  const end = Math.floor(script.samples.length / CHUNK) * CHUNK;

  for (let index = 0; index < end; index += 1) {
    if (script.speech[index] === 1) {
      spoken += 1;
      caught += heard[index] ?? 0;
    }
  }

  return caught / spoken;
}

/** A sentence of `words` words of about 350 ms, with 120-280 ms between them. */
function sentence(words: number, loudness: number, seed: number): ['w' | 'g', number, number?][] {
  const next = random(seed);
  const parts: ['w' | 'g', number, number?][] = [];

  for (let index = 0; index < words; index += 1) {
    parts.push(['w', 250 + next() * 250, loudness * (0.8 + next() * 0.4)]);

    if (index < words - 1) {
      parts.push(['g', 120 + next() * 160]);
    }
  }

  return parts;
}

describe('a long dictation', () => {
  it('hears every word of five minutes of continuous talk', () => {
    const parts: ['w' | 'g', number, number?][] = [['g', 800]];

    /* Five minutes: sentences of 6-20 words, short breaths between some, real pauses between others. */
    for (let index = 0, next = random(3); parts.reduce((sum, [, ms]) => sum + ms, 0) < 300_000; index += 1) {
      parts.push(...sentence(6 + Math.floor(next() * 14), 0.05, index + 11));
      parts.push(['g', next() < 0.5 ? 300 + next() * 300 : 1100 + next() * 1500]);
    }

    const script = record(parts);
    const { sent, offsets } = listen(script);

    expect(coverage(script, sent, offsets)).toBeGreaterThanOrEqual(0.999);
    expect(sent.length).toBeGreaterThan(20);
  });

  it('still hears a quiet voice after minutes of loud talk - the floor does not climb to the voice', () => {
    const parts: ['w' | 'g', number, number?][] = [['g', 800]];

    /* Three minutes of loud talk with only breaths in it, then soft-spoken sentences. */
    for (let index = 0; index < 60; index += 1) {
      parts.push(...sentence(10, 0.08, index + 100), ['g', 300]);
    }

    const softStart = parts.reduce((sum, [, ms]) => sum + ms, 0);

    for (let index = 0; index < 6; index += 1) {
      parts.push(...sentence(8, 0.014, index + 300), ['g', 1400]);
    }

    const script = record(parts);
    const { sent, offsets } = listen(script);
    const soft: Script = { samples: script.samples, speech: script.speech.map((value, index) => (index / RATE) * 1000 >= softStart ? value : 0) };

    expect(coverage(script, sent, offsets)).toBeGreaterThanOrEqual(0.999);
    expect(coverage(soft, sent, offsets)).toBeGreaterThanOrEqual(0.999);
  });

  it('cuts a breathless stretch before the limit, between words and never inside one', () => {
    /* Forty seconds with no pause longer than a word gap. */
    const parts: ['w' | 'g', number, number?][] = [['g', 600], ...sentence(80, 0.06, 41), ['g', 1500]];
    const script = record(parts);
    const { sent, offsets } = listen(script);

    expect(coverage(script, sent, offsets)).toBeGreaterThanOrEqual(0.999);
    expect(sent.length).toBeGreaterThanOrEqual(2);

    for (const phrase of sent) {
      expect((phrase.samples / RATE) * 1000).toBeLessThanOrEqual(MAX_PHRASE_MS + 200);
    }

    /* Each cut lands where the recording was not a word, or at worst at a word's quietest edge. */
    for (const phrase of sent.slice(0, -1)) {
      const lastChunk = phrase.chunks[phrase.chunks.length - 1] as Float32Array;
      const end = (offsets.get(lastChunk) ?? 0) + lastChunk.length;
      const around = script.speech.slice(Math.max(0, end - 2400), end + 2400);

      expect(around.includes(0)).toBe(true);
    }
  });

  it('sends nothing for a recording with nobody in it', () => {
    const { sent } = listen(record([['g', 20_000]], 0.006));

    expect(sent).toHaveLength(0);
  });

  it('keeps the start of the first word', () => {
    const script = record([['g', 3000], ...sentence(4, 0.05, 5), ['g', 1500]]);
    const { sent, offsets } = listen(script);

    expect(coverage(script, sent, offsets)).toBe(1);
  });
});

describe('tidyDictation', () => {
  const heard = (...texts: string[]) => texts.map((text) => ({ text, speechMs: 3000 }));

  it('joins phrases in order and keeps every word', () => {
    expect(tidyDictation(heard('Open the settings file', 'and change the port to 8080.'))).toBe('Open the settings file and change the port to 8080.');
  });

  it('takes out stutters and filler sounds', () => {
    expect(tidyDictation(heard('I I I want um to fix the the bug, uh, today.'))).toBe('I want to fix the bug, today.');
    expect(tidyDictation(heard('আমি উম এই ফাইলটা হুম ঠিক করতে চাই।'))).toBe('আমি এই ফাইলটা ঠিক করতে চাই।');
  });

  it('keeps a word doubled on purpose', () => {
    expect(tidyDictation(heard('ধীরে ধীরে লোড হচ্ছে, বার বার।'))).toBe('ধীরে ধীরে লোড হচ্ছে, বার বার।');
    expect(tidyDictation(heard('It is very very slow.'))).toBe('It is very very slow.');
    expect(tidyDictation(heard('আমি আমি আমি বলছি'))).toBe('আমি আমি বলছি');
  });

  it('drops words a cut left on both sides of it', () => {
    expect(tidyDictation(heard('the model dropdown should', 'dropdown should show the versions'))).toBe('the model dropdown should show the versions');
  });

  it('drops a thank-you invented for a near-silent phrase, never one that was said', () => {
    expect(tidyDictation([{ text: 'Fix the login.', speechMs: 2500 }, { text: 'Thank you.', speechMs: 300 }])).toBe('Fix the login.');
    expect(tidyDictation([{ text: 'ধন্যবাদ।', speechMs: 200 }])).toBe('');
    expect(tidyDictation([{ text: 'Fix the login.', speechMs: 2500 }, { text: 'Thank you.', speechMs: 1400 }])).toBe('Fix the login. Thank you.');
  });

  it('tidies the spaces around punctuation', () => {
    expect(tidyDictation(heard('Hello , world !', 'আমি ভালো আছি ।'))).toBe('Hello, world! আমি ভালো আছি।');
  });
});

describe('heldLanguage', () => {
  it('holds Bengali and Hindi, and leaves the rest to the transcriber', () => {
    expect(heldLanguage('আমি মডেল ড্রপডাউন ঠিক করতে চাই')).toBe('bn');
    expect(heldLanguage('मैं ठीक करना चाहता हूँ')).toBe('hi');
    expect(heldLanguage('fix the dropdown')).toBeUndefined();
    expect(heldLanguage('ok')).toBeUndefined();
  });
});
