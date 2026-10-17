# Voice input - diagnosis, research and the chain (Phase 1, 2026-10-18)

## 1. The root cause on the owner's Mac (SDC 3.0.3), with evidence

| # | Finding | Evidence |
|---|---|---|
| 1 | **macOS never grants the microphone to the app.** The installed `SDC.app` has no `NSMicrophoneUsageDescription` in `Info.plist` and is signed with the hardened runtime (`flags=0x10002(adhoc,runtime)`) with **no** `com.apple.security.device.audio-input` entitlement. Without both, macOS does not show the permission prompt and `getUserMedia` fails. | `PlistBuddy -c Print /Applications/SDC.app/Contents/Info.plist` (15 keys, none for the microphone); `codesign -dv --entitlements - /Applications/SDC.app` (no entitlements) |
| 2 | **The failure was swallowed.** `VoiceButton` caught every `getUserMedia` error and showed one sentence, "The microphone could not be opened", whatever the cause. | `app/src/kernel/VoiceButton.tsx` (3.0.3) |
| 3 | **The chain stopped at the first provider.** The old `transcribe` used the first cloud key it found and returned that provider's error. This Mac's keychain holds keys for `qwen`, `openai-api` and `deepseek`, and none for `groq` or `google` (checked with `security find-generic-password`, metadata only). A refused Alibaba key (HTTP 403, "unpurchased") ended the dictation without OpenAI ever being tried. | `intent/voice.rs` (3.0.3): `for ... { ... return heard.map(...) }` |
| 4 | **No local transcriber.** `whisper-cli`, `whisper-cpp` and `whisper` are not installed here, so there was no offline path. | `which` |

What changed (3.1): the Info.plist key and the entitlement (`app/src-tauri/Info.plist`, `Entitlements.plist`, `tauri.macos.conf.json`); every
microphone error is named with what to do (`lib/mic.ts`); the chain tries local whisper.cpp and then each cloud key in turn and reports every
failed attempt (`intent/voice.rs::transcribe_with`); a Gemini transcriber was added; the speech model downloads after a yes, with progress and a
SHA-256 check (`intent/voice_model.rs`); a spoken-language selector; `sdcd voice FILE.wav` runs the same chain from a terminal.

**Not provable from here:** that macOS shows the prompt and that a real microphone captures. That needs the rebuilt app on the owner's Mac
(OWNER-TO-RUN, see the final report).

## 2. Options checked (sources read 2026-10-18)

| Option | Free? | Bengali / Arabic / German | Offline | Privacy |
|---|---|---|---|---|
| whisper.cpp, local, `small-q5_1` (190 MB) | free | English 0 % WER, German 0 %, Arabic 25-33 %, **Bengali unusable** (measured below) | yes | stays on the machine |
| whisper.cpp, local, `large-v3-turbo-q5_0` (574 MB) | free | Arabic 0-42 %, **Bengali 50 % WER / 17 % CER with the language set, wrong script in auto mode** | yes | stays on the machine |
| Groq hosted Whisper large-v3 / turbo | free tier: 20 requests/min, 2,000/day, 7,200 audio-seconds/hour, 28,800/day (secondary sources; confirm at console.groq.com/docs/rate-limits) | Whisper-class | no | recording is sent to Groq |
| Google Gemini API, `gemini-3.5-transcribe` | "Free of charge" on ai.google.dev/gemini-api/docs/pricing; model page lists `bn-BD`, `bn-IN`, `ar-EG`, `de-DE`; up to 1 h per request | listed as supported; **not measured here (no key)** | no | recording is sent to Google |
| Alibaba Qwen3-ASR / OpenAI whisper-1 | paid / key needed | - | no | sent to the provider |
| DeepSeek, Claude, Brave | **no speech-to-text** | - | - | not used for audio |

Third-party figure: stock Whisper large-v3-turbo scored ~79 % normalised WER on Bengali before a community fine-tune brought it to ~11 %
(huggingface.co/mozilla-ai/whisper-large-v3-turbo-bn). That matches what was measured here: stock local Whisper is weak for Bengali.

**Decision:** local whisper.cpp first (private, offline, free; excellent for English/German, workable for Arabic), then Groq and Google Gemini
(both with a free key) and the Alibaba/OpenAI keys a person already has. For Bengali the chain should end on a cloud transcriber; the Settings
language selector exists because auto-detection called Bengali "Hindi".

## 3. Measured on generated clips (not human recordings)

Clips: `sdcd/tests/fixtures/voice/` (text-to-speech: macOS `say`; Edge neural voices for Arabic and Bengali). whisper.cpp built from source
(v1.9.5, Metal) on this Mac. WER/CER are word/character edit distances against the sentence that was synthesised, punctuation and Arabic
diacritics removed.

| Clip | small-q5_1, language set | turbo-q5_0, language set | turbo-q5_0, auto |
|---|---|---|---|
| English (`say` Samantha) | WER 0.00 | WER 0.00 | WER 0.00 |
| German (`say` Anna) | WER 0.00 | WER 0.00 | WER 0.00 |
| Arabic (`say` Majed) | WER 0.33, CER 0.11 | WER 0.42, CER 0.11 | WER 0.42, CER 0.11 |
| Arabic (neural) | WER 0.25, CER 0.02 | **WER 0.00** | **WER 0.00** |
| Bengali (`say` Piya - an English-accented voice) | WER 1.00 (output: commas) | WER 1.25, CER 1.02 (Devanagari) | WER 1.17 (Devanagari) |
| Bengali (neural) | WER 1.00 (Devanagari) | **WER 0.50, CER 0.17** | WER 1.17, CER 1.63 (Latin letters) |

Through SDC's own chain (`sdcd voice`, local turbo, language `bn`): 3.5 s for a 5.4 s clip, text `ওনুগ্রহ করে লগিন পেচটি খুলুন এবং পেমেন্ট মডিযুলের বারথো তেস্টি ঠিক কুরুন.`

Caveats, stated plainly: synthetic voices are cleaner than a real microphone in a real room, one sentence per language is a small sample, and
**the cloud transcribers were not measured** (no Groq or Google key on this Mac; the Alibaba/OpenAI keys cannot be read by a development
binary). The owner measures them with the command in the final report.

## 4. Owner-to-run

```
sdcd voice sdc/sdcd/tests/fixtures/voice/bn-neural.wav --lang bn --no-local     # cloud only, uses your keychain
sdcd voice sdc/sdcd/tests/fixtures/voice/ar-neural.wav --lang ar --no-local
```
The installed app's own copy of `sdcd` can read the keychain; a development build may be asked by macOS for permission.
