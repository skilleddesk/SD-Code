import { LoaderCircle, Mic, Square } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { heldLanguage, mergeAudio, MERGE_MAX_MS, MIN_PHRASE_MS, PhraseCutter, tidyDictation, TranscriberPacer, type HeardPhrase, type Phrase } from '../lib/dictation';
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
 *   - while a phrase is being spoken it is transcribed again every 1.2 s (on this computer) or 2.5 s (a cloud
 *     service, inside a budget per minute), so its words grow on screen (an interim caption: shown, never recorded);
 *   - when the phrase ends it is transcribed once more and its text is kept. 1.0.3: a transcriber that says "too many
 *     requests" is waited for as long as it asked and asked again - the phrases that piled up meanwhile go as one
 *     recording - so a dictation of many minutes loses nothing to a free key's rate limit; the next phrase starts
 *     empty, so a long dictation never re-sends what was already written;
 *   - when listening stops the phrases are put together by `tidyDictation`: stutters, filler sounds and words a
 *     cut left on both sides are taken out, and nothing else is changed.
 *
 * Every request is 16 kHz mono WAV, which whisper.cpp reads without `ffmpeg`; the daemon transcribes locally
 * when a whisper is installed and only otherwise with a connected provider, and says which one heard it.
 */

const RATE = 16000;
/** How long one transcription may take before it counts as lost. */
const HEAR_TIMEOUT_MS = 45_000;
/** How long a kept phrase is tried for - waits the transcriber asked for included - before its caption stands in. */
const GIVE_UP_MS = 180_000;
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

type Heard = { text: string; engine: string | null; local: boolean; error: string | null; retryAfterMs: number | null };

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
      resolve({ text: result.text, engine: result.engine, local: result.local, error: result.error, retryAfterMs: result.retryAfterMs ?? null });

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
  /* 1.0.6: while the last phrases are still being heard, a press gives up waiting - the button never holds anyone. */
  const abandonRef = useRef<(() => void) | null>(null);
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
    const pacer = new TranscriberPacer();
    const kept: HeardPhrase[] = [];
    let interim = '';
    /* The phrase the caption on screen belongs to. */
    let interimOf: Phrase | null = null;
    let interimBusy = false;
    let lastHeard: { engine: string | null; local: boolean } | null = null;
    /* Phrases waiting for their final text, oldest first. One worker sends them, in order. */
    const waiting: { slot: number; samples: Float32Array; speechMs: number }[] = [];
    let working: Promise<void> | null = null;
    /* The person's language - or, left on automatic, the one the first phrase was heard in, held for the rest. */
    let language = chosenLanguage();
    let warned = false;
    let listening = true;
    let abandoned = false;
    let waveAt = 0;

    const show = (): void => {
      onLive?.(tidyDictation([...kept, { text: interim, speechMs: Infinity }]));
    };

    const warnOnce = (message: string): void => {
      if (!warned) {
        warned = true;
        toast(message);
      }
    };

    /*
     * 1.0.3: the kept phrases' worker. The report was "it hears me for a short while and then stops" - a free key's
     * requests per minute ran out, every request after that was refused, and a phrase was given up after two tries.
     * Now a phrase is never given up because the transcriber is busy: when it asks to wait, the worker waits as long
     * as it asked, and the phrases that piled up meanwhile go together as one recording - one request, not ten.
     */
    const work = async (): Promise<void> => {
      while (waiting.length > 0) {
        const wait = pacer.waitFor(performance.now());

        if (wait > 0) {
          await new Promise((resolve) => window.setTimeout(resolve, wait));
        }

        /* Everything waiting, up to a minute of speech, as one request. */
        const batch: typeof waiting = [];
        let length = 0;

        while (waiting.length > 0 && (batch.length === 0 || length + (waiting[0]?.samples.length ?? 0) <= (MERGE_MAX_MS / 1000) * rate)) {
          const next = waiting.shift();

          if (next !== undefined) {
            batch.push(next);
            length += next.samples.length;
          }
        }

        const samples = batch.length === 1 ? (batch[0]?.samples ?? new Float32Array()) : mergeAudio(batch.map((part) => part.samples), rate);
        let heard: Heard | null = null;
        let failures = 0;
        const started = performance.now();

        for (;;) {
          pacer.note(performance.now());
          heard = await hear(samples, rate, sessionId, false, language);

          if (heard !== null && heard.error === null) {
            break;
          }

          const asked = heard?.retryAfterMs ?? null;

          /* 1.0.6: only a wait that ends inside the budget is waited for. A used-up day asks for minutes; waiting that out
             is the button that "spins and spins" - the reason is said at once instead. */
          if (asked !== null && asked > 0 && performance.now() - started + asked < GIVE_UP_MS && !abandoned) {
            /* Too many requests: rest as asked, then the same phrases again. */
            pacer.rest(performance.now(), asked + 250);
            await new Promise((resolve) => window.setTimeout(resolve, pacer.waitFor(performance.now())));
            continue;
          }

          failures += 1;

          if (failures >= 3 || performance.now() - started >= GIVE_UP_MS || abandoned) {
            break;
          }

          await new Promise((resolve) => window.setTimeout(resolve, failures * 1500));
        }

        const first = batch[0];

        if (heard !== null && heard.error === null && first !== undefined) {
          kept[first.slot] = { text: heard.text.trim(), speechMs: batch.reduce((sum, part) => sum + part.speechMs, 0) };

          for (const part of batch.slice(1)) {
            kept[part.slot] = { text: '', speechMs: part.speechMs };
          }

          lastHeard = { engine: heard.engine, local: heard.local };
          pacer.local = heard.local;
          language ??= heldLanguage(heard.text);
        } else {
          /* The captions stay, and everything said after it is still heard. */
          warnOnce(heard?.error ?? strings.kernel.voice.failed);
        }

        show();
      }

      working = null;
    };

    const finish = (ending: Phrase): void => {
      /* Until its final text arrives, the phrase keeps its last caption - so the words never blink out while the
         next phrase is already being captioned. A phrase that could not be heard keeps its caption too. */
      waiting.push({ slot: kept.length, samples: join(ending.chunks), speechMs: ending.speechMs });
      kept.push({ text: interimOf === ending ? interim : '', speechMs: ending.speechMs });

      if (interimOf === ending) {
        interim = '';
        interimOf = null;
      }

      working ??= work();
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

      /* A caption only while no kept phrase is waiting: the words that stay always go first. */
      if (current.speechMs > 0 && !interimBusy && waiting.length === 0 && lengthMs >= MIN_PHRASE_MS && pacer.mayCaption(now)) {
        const samples = join(current.chunks);

        interimBusy = true;
        void hear(samples, rate, sessionId, true, language).then((heard) => {
          interimBusy = false;

          if (heard?.retryAfterMs != null && heard.retryAfterMs > 0) {
            pacer.rest(performance.now(), heard.retryAfterMs);
          }

          if (heard !== null && heard.error === null) {
            pacer.local = heard.local;
          }

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
      processor.onaudioprocess = null;
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

      /* What was heard by the time the person gives up waiting is still put in the box. */
      const deliver = (): void => {
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
      };

      abandonRef.current = () => {
        abandonRef.current = null;
        abandoned = true;
        deliver();
      };

      const done = async (): Promise<void> => {
        while (working !== null) {
          await working;
        }
      };

      void done().then(() => {
        if (abandoned) {
          return;
        }

        abandonRef.current = null;
        deliver();
      });
    };

    /* A microphone that goes away (unplugged, taken by another app) ends the dictation the way Stop does, with
       everything heard so far kept - not a recording that silently hears nothing. */
    stream.getAudioTracks().forEach((track) => {
      track.addEventListener('ended', () => stopRef.current?.());
    });
  };

  const Icon = state === 'recording' ? Square : state === 'working' ? LoaderCircle : Mic;

  return (
    <span className={'inline-flex items-center gap-[4px] rounded-full transition-colors duration-fast ' + (state === 'recording' ? 'bg-red-subtle pl-[10px]' : '')}>
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
        'grid h-[30px] w-[30px] place-items-center rounded-full transition-all duration-fast ease-ease active:scale-[.94] ' +
        (state === 'recording' ? 'text-state-error hover:bg-state-error/15' : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary')
      }
      title={state === 'recording' ? `${strings.kernel.voice.listening} ${strings.kernel.voice.stop}` : state === 'working' ? strings.kernel.voice.stopWaiting : strings.kernel.voice.start}
      aria-label={state === 'recording' ? strings.kernel.voice.stop : strings.kernel.voice.start}
      aria-pressed={state === 'recording'}
      data-voice={state}
      onClick={() => {
        if (state === 'recording') {
          stopRef.current?.();
        } else if (state === 'working') {
          abandonRef.current?.();
        } else {
          void start();
        }
      }}
    >
      <Icon size={state === 'recording' ? 12 : 15} fill={state === 'recording' ? 'currentColor' : 'none'} aria-hidden="true" className={state === 'working' || state === 'recording' ? (state === 'working' ? 'animate-spin motion-reduce:animate-none' : 'animate-pulse motion-reduce:animate-none') : ''} />
    </button>
    </span>
  );
}
