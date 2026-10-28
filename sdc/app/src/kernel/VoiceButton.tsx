import { LoaderCircle, Mic, Square } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { heldLanguage, MIN_PHRASE_MS, PhraseCutter, tidyDictation, type HeardPhrase, type Phrase } from '../lib/dictation';
import { micProblem } from '../lib/mic';
import { waveLevel } from '../lib/voiceWave';
import { storedSettings } from '../lib/settings';
import { strings } from '../strings';
import { transcribe } from '../store/kernelIntents';
import { useAppStore } from '../store/store';
import { toast } from '../store/toast';

/**
 * Voice (0.12; live since 1.0): speak, and the words appear in the prompt box **while you speak** - then the
 * same Intent Engine as typing.
 *
 * It used to be tap, speak, tap, wait - and only then the whole text. The report: "kotha bolar somoy live chat
 * box a dakha jabe ki lakha hosse". No streaming recogniser is offered in every region (Model Studio's realtime
 * ASR is Beijing and Singapore only), so the live caption is built from the transcriber SDC already uses:
 *
 *   - the microphone is read as raw samples, cut into **phrases** at natural pauses (`lib/dictation`: 1 s of
 *     quiet after speech; a long breathless stretch is cut at its quietest moment, never inside a word);
 *   - while a phrase is being spoken it is transcribed again every ~1.2 s, so its words grow on screen
 *     (an interim caption: shown, never recorded - the daemon sends it live only);
 *   - when the phrase ends it is transcribed once more and its text is kept - with a time limit and one retry, so
 *     one lost answer can never hold back everything said after it; the next phrase starts empty, so a long
 *     dictation never re-sends what was already written;
 *   - when listening stops the phrases are put together by `tidyDictation`: stutters, filler sounds and words a
 *     cut left on both sides are taken out, and nothing else is changed.
 *
 * Every request is 16 kHz mono WAV, which whisper.cpp reads without `ffmpeg`; the daemon transcribes locally
 * when a whisper is installed and only otherwise with a connected provider, and says which one heard it.
 */

const RATE = 16000;
/** How often a phrase still being spoken is shown again. */
const INTERIM_MS = 1200;
/** How long one transcription may take before it counts as lost. */
const HEAR_TIMEOUT_MS = 45_000;
/** The bars of the live wave, and how often it is redrawn. */
const WAVE_BARS = 7;
const WAVE_MS = 60;

/** Samples, resampled to 16 kHz mono 16-bit WAV. */
function toWav(samples: Float32Array, rate: number): ArrayBuffer {
  const ratio = rate / RATE;
  const length = Math.floor(samples.length / ratio);
  const buffer = new ArrayBuffer(44 + length * 2);
  const view = new DataView(buffer);
  const text = (offset: number, value: string): void => {
    for (let index = 0; index < value.length; index += 1) {
      view.setUint8(offset + index, value.charCodeAt(index));
    }
  };

  text(0, 'RIFF');
  view.setUint32(4, 36 + length * 2, true);
  text(8, 'WAVE');
  text(12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, RATE, true);
  view.setUint32(28, RATE * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, 'data');
  view.setUint32(40, length * 2, true);

  for (let index = 0; index < length; index += 1) {
    /* The average of the source samples this one covers: a cheap low-pass, so the resample does not alias. */
    const from = Math.floor(index * ratio);
    const to = Math.max(from + 1, Math.floor((index + 1) * ratio));
    let sum = 0;

    for (let source = from; source < to && source < samples.length; source += 1) {
      sum += samples[source] ?? 0;
    }

    const clamped = Math.max(-1, Math.min(1, sum / (to - from)));

    view.setInt16(44 + index * 2, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
  }

  return buffer;
}

function base64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = '';

  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  }

  return btoa(binary);
}

function join(chunks: readonly Float32Array[]): Float32Array {
  const all = new Float32Array(chunks.reduce((sum, chunk) => sum + chunk.length, 0));
  let offset = 0;

  for (const chunk of chunks) {
    all.set(chunk, offset);
    offset += chunk.length;
  }

  return all;
}

type Heard = { text: string; engine: string | null; local: boolean; error: string | null };

/** One transcription, answered by the daemon's `VoiceTranscribed` event - or `null` when no answer came in time. */
async function hear(samples: Float32Array, rate: number, sessionId: string | undefined, interim: boolean, language: string | undefined): Promise<Heard | null> {
  const id = await transcribe(base64(toWav(samples, rate)), 'audio/wav', language, sessionId, interim, chosenEngine());

  if (id === null) {
    return null;
  }

  return new Promise((resolve) => {
    let stop = (): void => undefined;
    /* An answer that never arrives (a dropped event, a transcriber that hung) is a failure, not a wait forever. */
    const timer = window.setTimeout(() => {
      stop();
      resolve(null);
    }, HEAR_TIMEOUT_MS);
    const found = (): boolean => {
      const result = useAppStore.getState().kernel.voice[id];

      if (result === undefined) {
        return false;
      }

      window.clearTimeout(timer);
      resolve({ text: result.text, engine: result.engine, local: result.local, error: result.error });

      return true;
    };

    if (found()) {
      return;
    }

    stop = useAppStore.subscribe(() => {
      if (found()) {
        stop();
      }
    });
  });
}

/** The spoken language the person chose in Settings (`auto` = let the transcriber detect it). */
function chosenLanguage(): string | undefined {
  const value = storedSettings()['voice-language'];

  return typeof value === 'string' && value !== '' && value !== 'auto' ? value : undefined;
}

/** The transcriber the person chose in Settings (`auto` = the whole chain, so nothing is sent). */
function chosenEngine(): string | undefined {
  const value = storedSettings()['voice-engine'];

  return typeof value === 'string' && value !== '' && value !== 'auto' ? value : undefined;
}

/**
 * `onLive(text)` is the whole dictation so far (kept phrases plus the one being spoken) every time it changes;
 * `onText(text)` is the final dictation once listening stops. A caller that shows `onLive` should replace,
 * not append, on each call.
 */
export function VoiceButton({ onText, onLive, onStart, sessionId }: { onText: (text: string) => void; onLive?: (text: string) => void; onStart?: () => void; sessionId?: string }) {
  const [state, setState] = useState<'idle' | 'recording' | 'working'>('idle');
  const stopRef = useRef<(() => void) | null>(null);
  /* The last few loudness readings (0..1), newest last: the wave. Flat while nobody speaks. */
  const [wave, setWave] = useState<number[]>(() => new Array<number>(WAVE_BARS).fill(0));

  useEffect(() => () => stopRef.current?.(), []);

  const start = async (): Promise<void> => {
    let stream: MediaStream;

    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    } catch (error) {
      /* The reason, not a shrug: a blocked permission and a missing microphone need different fixes (Phase 1.1). */
      toast(micProblem(error));
      return;
    }

    const context = new AudioContext();

    /* A context made outside a gesture handler can start suspended (WebView2, WKWebView): no audio would flow. */
    if (context.state === 'suspended') {
      await context.resume().catch(() => undefined);
    }

    const source = context.createMediaStreamSource(stream);
    /* ScriptProcessor rather than an AudioWorklet: it needs no separate module file, and it is in every
       webview SDC runs in (WebView2, WKWebView, WebKitGTK). */
    const processor = context.createScriptProcessor(4096, 1, 1);
    const rate = context.sampleRate;
    const cutter = new PhraseCutter(rate);
    const kept: HeardPhrase[] = [];
    let interim = '';
    /* The phrase the caption on screen belongs to. */
    let interimOf: Phrase | null = null;
    let interimBusy = false;
    let interimAt = 0;
    let lastHeard: { engine: string | null; local: boolean } | null = null;
    /* Final transcriptions run in order, so the kept phrases stay in the order they were spoken. */
    let finals: Promise<void> = Promise.resolve();
    /* The person's language - or, left on automatic, the one the first phrase was heard in, held for the rest. */
    let language = chosenLanguage();
    let warned = false;
    let listening = true;
    let waveAt = 0;

    const show = (): void => {
      onLive?.(tidyDictation([...kept, { text: interim, speechMs: Infinity }]));
    };

    const finish = (ending: Phrase): void => {
      const samples = join(ending.chunks);
      const slot = kept.length;

      /* Until its final text arrives, the phrase keeps its last caption - so the words never blink out while the
         next phrase is already being captioned. A final that fails twice keeps the caption too. */
      kept.push({ text: interimOf === ending ? interim : '', speechMs: ending.speechMs });

      if (interimOf === ending) {
        interim = '';
        interimOf = null;
      }

      finals = finals.then(async () => {
        let heard = await hear(samples, rate, sessionId, false, language);

        /* One more try: a phrase is never given up on after a single lost answer. */
        if (heard === null || heard.error !== null) {
          heard = (await hear(samples, rate, sessionId, false, language)) ?? heard;
        }

        if (heard !== null && heard.error === null) {
          kept[slot] = { text: heard.text.trim(), speechMs: ending.speechMs };
          lastHeard = { engine: heard.engine, local: heard.local };
          language ??= heldLanguage(heard.text);
        } else if (!warned) {
          /* Said once per dictation: the caption stays, and everything after it is still heard. */
          warned = true;
          toast(heard?.error ?? strings.kernel.voice.failed);
        }

        show();
      });
    };

    processor.onaudioprocess = (event) => {
      if (!listening) {
        return;
      }

      const now = performance.now();
      const { ended, rms } = cutter.push(new Float32Array(event.inputBuffer.getChannelData(0)), now);

      if (now - waveAt >= WAVE_MS) {
        waveAt = now;

        const level = waveLevel(rms, cutter.floor);

        setWave((bars) => [...bars.slice(1), level]);
      }

      if (ended !== null) {
        finish(ended);
        return;
      }

      const current = cutter.phrase;
      const lengthMs = (current.samples / rate) * 1000;

      if (current.speechMs > 0 && !interimBusy && now - interimAt >= INTERIM_MS && lengthMs >= MIN_PHRASE_MS) {
        const samples = join(current.chunks);

        interimBusy = true;
        interimAt = now;
        void hear(samples, rate, sessionId, true, language).then((heard) => {
          interimBusy = false;

          /* Only while that phrase is still the one being spoken: its final text replaces a late caption. */
          if (heard !== null && heard.error === null && current === cutter.phrase) {
            interim = heard.text.trim();
            interimOf = current;
            show();
          }
        });
      }
    };

    source.connect(processor);
    processor.connect(context.destination);
    onStart?.();
    setState('recording');

    stopRef.current = () => {
      stopRef.current = null;
      listening = false;
      setWave(new Array<number>(WAVE_BARS).fill(0));
      processor.disconnect();
      source.disconnect();
      stream.getTracks().forEach((track) => track.stop());
      void context.close();

      /* The last phrase counts without a pause after it - but only if someone spoke in it: silence sent to a
         transcriber comes back as an invented "Thank you." */
      const ending = cutter.flush();

      if (ending !== null) {
        finish(ending);
      }

      setState('working');

      void finals.then(() => {
        setState('idle');

        const text = tidyDictation(kept);

        if (text === '') {
          toast(strings.kernel.voice.nothing);
          return;
        }

        onText(text);

        if (lastHeard !== null) {
          toast(lastHeard.local ? strings.kernel.voice.heardLocally(lastHeard.engine ?? '') : strings.kernel.voice.heardOnline(lastHeard.engine ?? ''));
        }
      });
    };
  };

  const Icon = state === 'recording' ? Square : state === 'working' ? LoaderCircle : Mic;

  return (
    <span className="inline-flex items-center gap-1">
      {state === 'recording' ? (
        <span className="flex h-[18px] items-center gap-[2px]" role="img" aria-label={strings.kernel.voice.listening} data-voice-wave>
          {wave.map((level, index) => (
            <span key={index} className="w-[3px] rounded-full bg-state-error transition-[height] duration-fast" style={{ height: `${Math.round(3 + level * 15)}px`, opacity: level > 0.05 ? 1 : 0.4 }} />
          ))}
        </span>
      ) : null}
    <button
      type="button"
      className={
        'grid h-[26px] w-[26px] place-items-center rounded-md transition-colors duration-fast ease-ease ' +
        (state === 'recording' ? 'bg-red-subtle text-state-error' : 'text-text-muted hover:bg-bg-hover hover:text-text-primary')
      }
      title={state === 'recording' ? `${strings.kernel.voice.listening} ${strings.kernel.voice.stop}` : strings.kernel.voice.start}
      aria-label={state === 'recording' ? strings.kernel.voice.stop : strings.kernel.voice.start}
      aria-pressed={state === 'recording'}
      data-voice={state}
      disabled={state === 'working'}
      onClick={() => {
        if (state === 'recording') {
          stopRef.current?.();
        } else {
          void start();
        }
      }}
    >
      <Icon size={14} aria-hidden="true" className={state === 'working' || state === 'recording' ? (state === 'working' ? 'animate-spin motion-reduce:animate-none' : 'animate-pulse motion-reduce:animate-none') : ''} />
    </button>
    </span>
  );
}
