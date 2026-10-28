/**
 * Dictation (v5): how a long stretch of speech is cut into phrases, and how the phrases' text is put back together.
 *
 * Pure on purpose - no microphone, no React - so the whole of it is tested with recorded-like signals
 * (`dictation.test.ts`). `VoiceButton` feeds it the microphone and the transcriber.
 *
 * The report it answers: "a long dictation misses parts". Three causes, each fixed here:
 *
 *   1. **The noise floor crept up to the voice.** It was learnt from every chunk, speech included, so after about a
 *      minute of talking the threshold sat above the voice and whole phrases were judged silent - and dropped.
 *      The floor now learns from quiet only (`VoiceDetector`).
 *   2. **Phrases were cut in the middle of words** at the 20 s limit. A long phrase is now cut at its quietest
 *      moment near the limit (`PhraseCutter`), and the pause that ends a phrase is a natural one (1 s, not 0.7 s), so
 *      the transcriber hears whole sentences.
 *   3. **What was heard was glued together as-is.** `tidyDictation` takes out stutters, filler sounds, words
 *      repeated across a cut, and the "Thank you." a transcriber invents for a near-silent phrase.
 */

/** The pause that ends a phrase. */
export const PAUSE_MS = 1000;
/** Past this a phrase ends at its next short quiet moment. */
export const SOFT_MAX_MS = 18_000;
/** A short quiet moment, enough to cut a long phrase between words. */
export const SHORT_PAUSE_MS = 250;
/** The longest phrase: cut here even without a pause, at the quietest moment of its last stretch. */
export const MAX_PHRASE_MS = 28_000;
/** How far back the forced cut looks for its quietest moment. */
const CUT_SEARCH_MS = 2_000;
/** A phrase shorter than this is not worth a request. */
export const MIN_PHRASE_MS = 400;
/** Before anyone speaks only this much is kept: the start of the first word. */
export const PREROLL_MS = 600;

/**
 * Is this chunk speech? A noise floor learnt from the quiet, and a threshold above it.
 *
 * The floor falls fast (a quieter room is believed at once), rises slowly while nobody speaks (a fan turned on),
 * and all but stops rising while someone does - which is the fix: before, it followed the voice up.
 */
export class VoiceDetector {
  floor = 0.004;
  private speaking = false;

  /** The level a chunk must pass to start speech, and the lower one it must stay above to continue it. */
  threshold(): number {
    return Math.max(0.006, this.floor * 2.5);
  }

  /** The lower level that keeps speech going once it started: the tail of a word is quieter than its start. */
  holds(): number {
    return Math.max(0.004, this.floor * 1.5);
  }

  hear(rms: number): boolean {
    this.speaking = rms > (this.speaking ? this.holds() : this.threshold());

    if (rms < this.floor) {
      this.floor = rms * 0.5 + this.floor * 0.5;
    } else if (!this.speaking && rms < this.floor * 2) {
      /* Sounds like the room: learnt in a second or two. */
      this.floor = this.floor * 0.98 + rms * 0.02;
    } else if (!this.speaking) {
      /* Between the room and a voice - the trough between two syllables, as often as not: learnt over tens of
         seconds, so a soft speaker's own words cannot lift the threshold above them. */
      this.floor = this.floor * 0.998 + rms * 0.002;
    } else {
      /* A noise that never stops is learnt in the end - over minutes, not over a sentence. */
      this.floor = this.floor * 0.99995 + rms * 0.00005;
    }

    this.floor = Math.max(0.0005, this.floor);

    return this.speaking;
  }
}

/** A phrase being recorded: its chunks with their loudness, and how much of it was speech. */
export interface Phrase {
  chunks: Float32Array[];
  levels: number[];
  samples: number;
  /** Milliseconds judged as speech. */
  speechMs: number;
  quietSince: number | null;
}

export const freshPhrase = (): Phrase => ({ chunks: [], levels: [], samples: 0, speechMs: 0, quietSince: null });

/**
 * Cuts the microphone into phrases. `push` takes one chunk and answers the phrases it finished (zero or one),
 * each of them worth sending: someone spoke in it, and it is long enough to be words.
 */
export class PhraseCutter {
  phrase: Phrase = freshPhrase();
  private readonly detector = new VoiceDetector();

  constructor(private readonly rate: number) {}

  private ms(samples: number): number {
    return (samples / this.rate) * 1000;
  }

  /** The level of the voice detector's floor, for the live wave. */
  get floor(): number {
    return this.detector.floor;
  }

  push(chunk: Float32Array, now: number): { ended: Phrase | null; speaking: boolean; rms: number } {
    let energy = 0;

    for (const sample of chunk) {
      energy += sample * sample;
    }

    const rms = Math.sqrt(energy / Math.max(1, chunk.length));
    const speaking = this.detector.hear(rms);
    const phrase = this.phrase;

    phrase.chunks.push(chunk);
    phrase.levels.push(rms);
    phrase.samples += chunk.length;

    if (speaking) {
      phrase.speechMs += this.ms(chunk.length);
      phrase.quietSince = null;
    } else if (phrase.quietSince === null) {
      phrase.quietSince = now;
    }

    const spoke = phrase.speechMs > 0;
    const length = this.ms(phrase.samples);
    const quiet = phrase.quietSince === null ? 0 : now - phrase.quietSince;

    /* Nobody spoke yet: keep only the pre-roll, the start of the first word. */
    if (!spoke) {
      while (phrase.chunks.length > 1 && this.ms(phrase.samples - (phrase.chunks[0]?.length ?? 0)) >= PREROLL_MS) {
        phrase.samples -= phrase.chunks.shift()?.length ?? 0;
        phrase.levels.shift();
      }

      return { ended: null, speaking, rms };
    }

    if (quiet >= PAUSE_MS || (length >= SOFT_MAX_MS && quiet >= SHORT_PAUSE_MS)) {
      return { ended: this.cut(phrase.chunks.length), speaking, rms };
    }

    if (length >= MAX_PHRASE_MS) {
      return { ended: this.cut(this.quietestIndex() + 1), speaking, rms };
    }

    return { ended: null, speaking, rms };
  }

  /** Ends the phrase being recorded (the person pressed stop). */
  flush(): Phrase | null {
    const ending = this.phrase;

    this.phrase = freshPhrase();

    return worthSending(ending, this.rate) ? ending : null;
  }

  /** The chunk to cut after: the quietest of the phrase's last couple of seconds, so no word is split. */
  private quietestIndex(): number {
    const { chunks, levels } = this.phrase;
    let index = chunks.length - 1;
    let span = 0;
    let best = index;

    while (index > 0 && span < CUT_SEARCH_MS) {
      span += this.ms(chunks[index]?.length ?? 0);

      if ((levels[index] ?? Infinity) <= (levels[best] ?? Infinity)) {
        best = index;
      }

      index -= 1;
    }

    return best;
  }

  /** Ends the phrase after its first `count` chunks; the rest start the next one, nothing is dropped. */
  private cut(count: number): Phrase | null {
    const phrase = this.phrase;
    const ending: Phrase = { ...freshPhrase(), chunks: phrase.chunks.slice(0, count), levels: phrase.levels.slice(0, count) };
    const rest: Phrase = { ...freshPhrase(), chunks: phrase.chunks.slice(count), levels: phrase.levels.slice(count) };

    ending.samples = ending.chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    rest.samples = rest.chunks.reduce((sum, chunk) => sum + chunk.length, 0);

    /* A forced cut shares the speech out by the chunks each side keeps that were loud enough to be speech. */
    const share = (part: Phrase): number =>
      part.levels.reduce((sum, level, at) => sum + (level > this.detector.holds() ? this.ms(part.chunks[at]?.length ?? 0) : 0), 0);

    ending.speechMs = rest.chunks.length === 0 ? phrase.speechMs : share(ending);
    rest.speechMs = rest.chunks.length === 0 ? 0 : share(rest);
    this.phrase = rest;

    return worthSending(ending, this.rate) ? ending : null;
  }
}

/** Someone spoke in it, and it is long enough to be words: silence sent to a transcriber comes back invented. */
export function worthSending(phrase: Phrase, rate: number): boolean {
  return phrase.speechMs >= MIN_SPEECH_MS && (phrase.samples / rate) * 1000 >= MIN_PHRASE_MS;
}

/** Less speech than this in a phrase is a click or a cough, not a word. */
export const MIN_SPEECH_MS = 120;

/* ---- Putting the words back together ------------------------------------------------------------------------ */

/** One transcribed phrase, and how much of its audio was speech. */
export interface HeardPhrase {
  text: string;
  speechMs: number;
}

/**
 * What transcribers write for a near-silent phrase (Whisper's training subtitles). Dropped only when the phrase had
 * almost no speech in it - "thank you" said on purpose, in a phrase of its own, is longer than this.
 */
const INVENTED = [
  'thank you',
  'thank you very much',
  'thanks for watching',
  'thank you for watching',
  'please subscribe',
  'subtitles by the amara org community',
  'you',
  'bye',
  'ধন্যবাদ',
  'শুভেচ্ছা',
  'धन्यवाद',
  'شكرا',
  'شکریہ',
];
const INVENTED_BELOW_MS = 900;

/** Sounds, not words. Matched as whole words only. */
const FILLERS = new Set(['um', 'umm', 'ummm', 'uh', 'uhh', 'uhm', 'erm', 'er', 'hmm', 'hmmm', 'mm', 'mmm', 'উম', 'উমম', 'উঁ', 'আঁ', 'এ্যা', 'অ্যা', 'হুম', 'হুমম', 'উহ', 'उम', 'उह', 'हम्म', 'امم']);

/** Doubled on purpose in English; any other doubled Latin word is a stutter. */
const DOUBLE_OK = new Set(['very', 'so', 'bye', 'no', 'yes', 'ha', 'really', 'that', 'had', 'is', 'more', 'again', 'many', 'far', 'now', 'go', 'well']);

/** A word as it is compared: lower case, without the punctuation around it. */
function bare(word: string): string {
  return word.toLowerCase().replace(/^[\s"'“”‘’(.,!?;:।॥،؟-]+|[\s"'“”‘’).,!?;:।॥،؟-]+$/g, '');
}

/** Bengali, Devanagari, Arabic script: a doubled word is grammar there (`ধীরে ধীরে`, `बार बार`), not a stutter. */
const REDUPLICATING = /[\p{Script=Bengali}\p{Script=Devanagari}\p{Script=Arabic}]/u;

function tidyWords(words: string[]): string[] {
  const out: string[] = [];

  for (const word of words) {
    const key = bare(word);

    if (key !== '' && FILLERS.has(key)) {
      /* A filler that carried the sentence's full stop hands it to the word before it. */
      const tail = /[.!?।]$/.exec(word)?.[0];

      if (tail !== undefined && out.length > 0 && !/[.!?।]$/.test(out[out.length - 1] ?? '')) {
        out[out.length - 1] = (out[out.length - 1] ?? '').replace(/[,;:]$/, '') + tail;
      }

      continue;
    }

    /* How many times this word already stands at the end of the output. */
    let run = 0;

    for (let at = out.length - 1; at >= 0 && bare(out[at] ?? '') === key && key !== ''; at -= 1) {
      run += 1;
    }

    const allowedRun = REDUPLICATING.test(key) || DOUBLE_OK.has(key) || /^\d/.test(key) ? 2 : 1;

    if (run >= allowedRun) {
      /* The stutter's copies go; the punctuation the last copy carried stays. */
      const tail = /[.,!?;:।]+$/.exec(word)?.[0];

      if (tail !== undefined) {
        out[out.length - 1] = (out[out.length - 1] ?? '').replace(/[.,!?;:।]+$/, '') + tail;
      }

      continue;
    }

    out.push(word);
  }

  return out;
}

/**
 * The phrases of one dictation as one clean text.
 *
 * Kept: every word the person said, in order. Taken out: invented thank-yous on near-silent phrases, filler sounds,
 * stutters (`I I I want` -> `I want`; doubled words in Bengali/Hindi/Arabic script are grammar and stay), words a cut
 * left on both sides of it, and spaces before punctuation.
 */
export function tidyDictation(phrases: readonly HeardPhrase[]): string {
  const words: string[] = [];

  for (const phrase of phrases) {
    const text = phrase.text.replace(/\s+/g, ' ').trim();

    if (text === '') {
      continue;
    }

    const normalised = bare(text.replace(/[.,!?;:।]/g, ' ').replace(/\s+/g, ' '));

    if (phrase.speechMs < INVENTED_BELOW_MS && INVENTED.includes(normalised)) {
      continue;
    }

    let incoming = text.split(' ');

    /* A cut heard twice: the phrase starts with the words the last one ended on. */
    for (let overlap = Math.min(4, words.length, incoming.length - 1); overlap >= 1; overlap -= 1) {
      const tail = words.slice(-overlap).map(bare);
      const head = incoming.slice(0, overlap).map(bare);

      if (tail.every((word, index) => word !== '' && word === head[index]) && (overlap > 1 || (tail[0]?.length ?? 0) > 3)) {
        incoming = incoming.slice(overlap);
        break;
      }
    }

    words.push(...incoming);
  }

  return tidyWords(words)
    .join(' ')
    .replace(/\s+([.,!?;:।॥،؟])/g, '$1')
    .replace(/([,;:])(?=[^\s\d])/g, '$1 ')
    .replace(/\s+/g, ' ')
    .trim();
}

/**
 * The language to hold a dictation to once its first phrase is heard, when the person left it on automatic: a
 * transcriber that detects the language per phrase flips a Bengali speaker's short phrase into Hindi or Urdu script,
 * which is the "inconsistency" a reader sees. Only the scripts that name one language are held.
 */
export function heldLanguage(text: string): string | undefined {
  const letters = text.replace(/[\s\d\p{P}]/gu, '');

  if (letters.length < 4) {
    return undefined;
  }

  const share = (pattern: RegExp): number => (letters.match(pattern)?.length ?? 0) / letters.length;

  if (share(/\p{Script=Bengali}/gu) > 0.5) {
    return 'bn';
  }

  if (share(/\p{Script=Devanagari}/gu) > 0.5) {
    return 'hi';
  }

  return undefined;
}
