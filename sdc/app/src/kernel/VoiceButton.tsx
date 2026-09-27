import { LoaderCircle, Mic, Square } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { strings } from '../strings';
import { transcribe } from '../store/kernelIntents';
import { useAppStore } from '../store/store';
import { toast } from '../store/toast';

/**
 * Voice (0.12): speak, and the words land in the prompt box - then the same Intent Engine as typing.
 *
 * The recording is turned into 16 kHz mono WAV here, in the window, because that is what whisper.cpp
 * reads without `ffmpeg`: the daemon transcribes locally when a whisper is installed and only otherwise
 * with a connected provider, and says which one heard it.
 */

/** Any recording, decoded and resampled to 16 kHz mono 16-bit WAV. */
async function toWav(blob: Blob): Promise<ArrayBuffer> {
  const decoded = await new AudioContext().decodeAudioData(await blob.arrayBuffer());
  const rate = 16000;
  const offline = new OfflineAudioContext(1, Math.ceil(decoded.duration * rate), rate);
  const source = offline.createBufferSource();

  source.buffer = decoded;
  source.connect(offline.destination);
  source.start();

  const samples = (await offline.startRendering()).getChannelData(0);
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const text = (offset: number, value: string): void => {
    for (let index = 0; index < value.length; index += 1) {
      view.setUint8(offset + index, value.charCodeAt(index));
    }
  };

  text(0, 'RIFF');
  view.setUint32(4, 36 + samples.length * 2, true);
  text(8, 'WAVE');
  text(12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, 'data');
  view.setUint32(40, samples.length * 2, true);

  samples.forEach((sample, index) => {
    const clamped = Math.max(-1, Math.min(1, sample));

    view.setInt16(44 + index * 2, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
  });

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

export function VoiceButton({ onText, sessionId }: { onText: (text: string) => void; sessionId?: string }) {
  const [state, setState] = useState<'idle' | 'recording' | 'working'>('idle');
  const [request, setRequest] = useState<string | null>(null);
  const recorder = useRef<MediaRecorder | null>(null);
  const chunks = useRef<Blob[]>([]);
  const result = useAppStore((store) => (request === null ? undefined : store.kernel.voice[request]));

  useEffect(() => {
    if (result === undefined) {
      return;
    }

    setRequest(null);
    setState('idle');

    if (result.error !== null) {
      toast(result.error);
    } else if (result.text.trim() !== '') {
      onText(result.text.trim());
      toast(result.local ? strings.kernel.voice.heardLocally(result.engine ?? '') : strings.kernel.voice.heardOnline(result.engine ?? ''));
    } else {
      toast(strings.kernel.voice.nothing);
    }
  }, [result, onText]);

  const start = async (): Promise<void> => {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const media = new MediaRecorder(stream);

      chunks.current = [];
      media.ondataavailable = (event) => chunks.current.push(event.data);
      media.onstop = () => {
        stream.getTracks().forEach((track) => track.stop());
        setState('working');

        void toWav(new Blob(chunks.current, { type: media.mimeType }))
          .then((wav) => transcribe(base64(wav), 'audio/wav', undefined, sessionId))
          .then((id) => {
            if (id === null) {
              setState('idle');
            } else {
              setRequest(id);
            }
          })
          .catch(() => {
            setState('idle');
            toast(strings.kernel.voice.failed);
          });
      };
      recorder.current = media;
      media.start();
      setState('recording');
    } catch {
      toast(strings.kernel.voice.noMicrophone);
    }
  };

  const Icon = state === 'recording' ? Square : state === 'working' ? LoaderCircle : Mic;

  return (
    <button
      type="button"
      className={
        'grid h-[26px] w-[26px] place-items-center rounded-md transition-colors duration-fast ease-ease ' +
        (state === 'recording' ? 'bg-red-subtle text-state-error' : 'text-text-muted hover:bg-bg-hover hover:text-text-primary')
      }
      title={state === 'recording' ? strings.kernel.voice.stop : strings.kernel.voice.start}
      aria-label={state === 'recording' ? strings.kernel.voice.stop : strings.kernel.voice.start}
      aria-pressed={state === 'recording'}
      disabled={state === 'working'}
      onClick={() => {
        if (state === 'recording') {
          recorder.current?.stop();
        } else {
          void start();
        }
      }}
    >
      <Icon size={14} aria-hidden="true" className={state === 'working' ? 'animate-spin motion-reduce:animate-none' : ''} />
    </button>
  );
}
