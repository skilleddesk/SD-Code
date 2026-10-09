import { LoaderCircle, Mic, Square } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

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
 *   - the microphone is read as raw samples, cut into **phrases** at the pauses (700 ms of quiet after
 *     speech, or 20 s at most);
 *   - while a phrase is being spoken it is transcribed again every ~1.2 s, so its words grow on screen
 *     (an interim caption: shown, never recorded - the daemon sends it live only);
 *   - when the phrase ends it is transcribed once more and its text is kept; the next phrase starts empty,
 *     so a long dictation never re-sends what was already written.
 *
 * Every request is 16 kHz mono WAV, which whisper.cpp reads without `ffmpeg`; the daemon transcribes locally
 * when a whisper is installed and only otherwise with a connected provider, and says which one heard it.
 */

const RATE = 16000;
/** How often a phrase still being spoken is shown again. */
const INTERIM_MS = 1200;
/** The pause that ends a phrase. */
const PAUSE_MS = 700;
/** The longest phrase: past this it is cut even without a pause, so no request grows without a bound. */
const MAX_PHRASE_MS = 20_000;
/** A phrase shorter than this is not worth a request. */
const MIN_PHRASE_MS = 400;

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

/** One transcription, answered by the daemon's `VoiceTranscribed` event. */
async function hear(samples: Float32Array, rate: number, sessionId: string | undefined, interim: boolean): Promise<{ text: string; engine: string | null; local: boolean; error: string | null } | null> {
  const id = await transcribe(base64(toWav(samples, rate)), 'audio/wav', undefined, sessionId, interim);

  if (id === null) {
    return null;
  }

  return new Promise((resolve) => {
    const found = (): boolean => {
      const result = useAppStore.getState().kernel.voice[id];

      if (result === undefined) {
        return false;
      }

      resolve({ text: result.text, engine: result.engine, local: result.local, error: result.error });

      return true;
    };

    if (found()) {
      return;
    }

    const stop = useAppStore.subscribe(() => {
      if (found()) {
        stop();
      }
    });
  });
}

/** A phrase being recorded: its samples, whether anyone spoke in it, and when they last did. */
interface Phrase {
  chunks: Float32Array[];
  samples: number;
  spoke: boolean;
  quietSince: number | null;
}

const fresh = (): Phrase => ({ chunks: [], samples: 0, spoke: false, quietSince: null });

/**
 * `onLive(text)` is the whole dictation so far (kept phrases plus the one being spoken) every time it changes;
 * `onText(text)` is the final dictation once listening stops. A caller that shows `onLive` should replace,
 * not append, on each call.
 */
export function VoiceButton({ onText, onLive, onStart, sessionId }: { onText: (text: string) => void; onLive?: (text: string) => void; onStart?: () => void; sessionId?: string }) {
  const [state, setState] = useState<'idle' | 'recording' | 'working'>('idle');
  const stopRef = useRef<(() => void) | null>(null);

  useEffect(() => () => stopRef.current?.(), []);

  const start = async (): Promise<void> => {
    let stream: MediaStream;

    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
    } catch {
      toast(strings.kernel.voice.noMicrophone);
      return;
    }

    const context = new AudioContext();
    const source = context.createMediaStreamSource(stream);
    /* ScriptProcessor rather than an AudioWorklet: it needs no separate module file, and it is in every
       webview SDC runs in (WebView2, WKWebView, WebKitGTK). */
    const processor = context.createScriptProcessor(4096, 1, 1);
    const rate = context.sampleRate;
    const kept: string[] = [];
    let phrase = fresh();
    let interim = '';
    /* The phrase the caption on screen belongs to. */
    let interimOf: Phrase | null = null;
    let interimBusy = false;
    let interimAt = 0;
    let lastHeard: { engine: string | null; local: boolean } | null = null;
    /* Final transcriptions run in order, so the kept phrases stay in the order they were spoken. */
    let finals: Promise<void> = Promise.resolve();
    /* A noise floor learnt from the quietest chunks: speech is well above it. */
    let floor = 0.004;
    let listening = true;

    const show = (): void => {
      onLive?.([...kept, interim].filter((part) => part.trim() !== '').join(' '));
    };

    const finish = (ending: Phrase): void => {
      if (!ending.spoke || (ending.samples / rate) * 1000 < MIN_PHRASE_MS) {
        return;
      }

      const samples = join(ending.chunks);
      const slot = kept.length;

      /* Until its final text arrives, the phrase keeps its last caption - so the words never blink out while the
         next phrase is already being captioned (seen live: half a second with a sentence missing). A final that
         fails keeps the caption too. */
      kept.push(interimOf === ending ? interim : '');

      if (interimOf === ending) {
        interim = '';
        interimOf = null;
      }

      finals = finals.then(async () => {
        const heard = await hear(samples, rate, sessionId, false);

        if (heard !== null && heard.error === null) {
          kept[slot] = heard.text.trim();
          lastHeard = { engine: heard.engine, local: heard.local };
        } else if (heard?.error) {
          toast(heard.error);
        }

        show();
      });
    };

    processor.onaudioprocess = (event) => {
      if (!listening) {
        return;
      }

      const input = event.inputBuffer.getChannelData(0);
      const chunk = new Float32Array(input);
      let energy = 0;

      for (const sample of chunk) {
        energy += sample * sample;
      }

      const rms = Math.sqrt(energy / chunk.length);
      const now = performance.now();

      floor = rms < floor ? rms * 0.5 + floor * 0.5 : floor * 0.999 + rms * 0.001;

      const speaking = rms > Math.max(0.012, floor * 3);

      phrase.chunks.push(chunk);
      phrase.samples += chunk.length;

      if (speaking) {
        phrase.spoke = true;
        phrase.quietSince = null;
      } else if (phrase.quietSince === null) {
        phrase.quietSince = now;
      }

      const lengthMs = (phrase.samples / rate) * 1000;
      const paused = phrase.spoke && phrase.quietSince !== null && now - phrase.quietSince >= PAUSE_MS;

      if (paused || lengthMs >= MAX_PHRASE_MS) {
        const ending = phrase;

        phrase = fresh();
        finish(ending);

        return;
      }

      /* Nobody spoke yet: keep only the last half second, the start of the first word. */
      if (!phrase.spoke && lengthMs > 500) {
        const drop = phrase.chunks.shift();

        phrase.samples -= drop?.length ?? 0;
      }

      if (phrase.spoke && !interimBusy && now - interimAt >= INTERIM_MS && lengthMs >= MIN_PHRASE_MS) {
        const current = phrase;
        const samples = join(current.chunks);

        interimBusy = true;
        interimAt = now;
        void hear(samples, rate, sessionId, true).then((heard) => {
          interimBusy = false;

          /* Only while that phrase is still the one being spoken: its final text replaces a late caption. */
          if (heard !== null && heard.error === null && current === phrase) {
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
      processor.disconnect();
      source.disconnect();
      stream.getTracks().forEach((track) => track.stop());
      void context.close();

      const ending = phrase;

      phrase = fresh();
      /* The last phrase counts without a pause after it - but only if someone spoke in it: silence sent to a
         transcriber comes back as an invented "Thank you." */
      finish(ending);
      setState('working');

      void finals.then(() => {
        setState('idle');

        const text = kept.filter((part) => part !== '').join(' ').trim();

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
  );
}
