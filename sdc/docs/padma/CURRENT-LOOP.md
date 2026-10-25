# PADMA kickoff - the agent loop as it was on 25 Oct 2026 (before any PADMA code)

This is the map the kickoff prompt asks for. It describes the loop **before** PADMA; no behaviour was changed to write it. Paths are
relative to `sdc/sdcd/src/`; line numbers are those of the tree at kickoff (`git status` was clean).

## 1. Where a turn ends

| What | Where |
| ---- | ----- |
| The loop: `for step in 1..=options.max_steps` (the global "25 steps per turn" cap, `DEFAULT_STEPS`) | `agent/mod.rs:2191` |
| A reply with **zero tool calls** is the end of the turn: `if reply.tool_uses.is_empty() { ... EngineEvent::Done { summary: "Done" } ... return }` | `agent/mod.rs:2621-2751` |
| Nothing but the model decides this: before it there are only *nudges* that each fire once (plan nudge 2549, auto-check 2562, self-review 2576, three-part contract 2586) | `agent/mod.rs:2528-2592` |
| An empty reply: one retry with thinking off (`empty_retries`), then `Failed("... returned an empty answer ...")` | `agent/mod.rs:2596-2619` |
| The step executor (NAVARATNA) is a separate road out: `steps::run(...)` then `return` | `agent/mod.rs:2081-2102` |

**Finding (RC1):** a reply ending in `:` with no tool call reaches line 2621 and is reported `Done`. Nothing looks at what the reply *says*.

## 2. Where tool calls are read from the model's answer

| Step | Where |
| ---- | ----- |
| Ollama's NDJSON is translated to SSE; `message.thinking` becomes `reasoning_content`, `message.tool_calls` becomes `tool_calls` deltas | `agent/ollama_native.rs:146-147` |
| The stream is assembled into a `Reply` (`OpenAiState::feed` / `finish`) - **`reasoning_content` is counted (`thinking_tokens`) but its text is dropped** | `agent/dialect.rs:658`, `:738`, `Reply` at `:39` |
| Text-form tool calls (`<tool_call>`, XML, bare JSON) are read by `rescue::apply` - **only from `reply.text`, never from thinking, and only when `sme.toolcall_rescue` is on** | `agent/mod.rs:2426-2437`, `agent/rescue.rs:396` (`rescue`), `:515` (`apply`) |

**Finding (RC2):** a call written inside the thinking field, or a model that answers with thinking only, is invisible to the loop.

## 3. Where `ContextOverflow` is raised and what happens to it

| Step | Where |
| ---- | ----- |
| The guard: `guard::context::check(&body, window, reserve)` -> `Err(ContextOverflow)` -> `ask_model` returns `Err(text)` | `agent/mod.rs:677-686`, `guard/context.rs:75` |
| N1-1: `navaratna::window::usable_for` shrinks the window; the error text starts `ContextOverflow: NeedsSplit` | `agent/mod.rs:665-667`, `navaratna/window.rs:15-28` |
| The loop's handling: `Err(reason) if too_long(&reason) && !shrunk` folds old results once (`fold_old_results(.., 2)`) and asks again | `agent/mod.rs:2312-2319` |
| Everything else falls to the last arm: `sink.send(EngineEvent::Failed(reason)); return;` - **the whole turn dies** | `agent/mod.rs:2346-2392` |
| The error is shown by the translator with the fallback "This failure has no rule yet" | `errors/translator.rs:253` |

**Finding (RC4), a defect found while mapping:** `too_long()` (`agent/mod.rs:992`) matches provider phrases such as "maximum context length",
but **not SDC's own `ContextOverflow:` text**. So the guard's refusal never even took the one fold-and-retry; it went straight to
`Failed`. 12,397 + 4,096 reserve = 16,493 > 16,384 (109 over) ended six turns of work.

There is no per-observation cap anywhere in the loop (RC3): every tool output is appended whole to `messages`.

## 4. Where UI notices are appended

| Notice | Where |
| ------ | ----- |
| Slow-model warning (`SpillWatch`) | `agent/mod.rs:2230-2232` -> `EngineEvent::Delta("_(..)_")` |
| GPU-offload notice (`spill_warning`, "Only 86% of this model is on the GPU") | `agent/mod.rs:2239-2241` -> `EngineEvent::Delta("_(..)_")`; text at `sme/tuner.rs:503` |
| Interrupted-step notices | `agent/mod.rs:535`, `:542` -> `Delta` |

`Delta` is the answer text. `sdcp/methods.rs` accumulates every `Delta` into `answer` and stores it with `finish_turn` (~`:735`); the next
turn reads it back through `session_bridge::history_for` (`session_bridge/mod.rs:25`). **A notice sent as `Delta` is therefore part of what
the model is told it said last turn** (RC10).

## 5. Where Trust is computed

`sdcp/methods.rs:932` (`score_turn`) -> `trust::score::facts` (`trust/score.rs:37`) -> `trust::score::turn` (`trust/score.rs:106`). `turn()` starts
at 100 and only **subtracts** for policy hits, secrets, SAST, size and failed checks. A turn that did nothing has nothing to subtract: 100,
"high" (RC9). `trust/proof.rs:77` builds the same score for the proof file.

## 6. Which switches gate the NAVARATNA step executor

`agent/steps.rs:28` `applies(...)`:

```
backend == Ollama && !remote && !research && autonomy == Auto
  && request.len() >= navaratna::spec::BIG_PROMPT_CHARS (1,500 characters)   <- prompt LENGTH (RC5)
  && v3::on_in("nr.fresh_step", scope)                                       <- default OFF (RC6)
```

Inside it: `nr.sentence_cover` (on), `nr.lang_spec`, `nr.blueprint`, `nr.test_first`, `nr.best_of_n`, `nr.out_chunk`, `nr.git_shadow` (all off).
`nr.tool_envelope` (off) gates the prose-instead-of-tool re-ask at `agent/mod.rs:2510-2521` and `navaratna/envelope.rs`. `nr.error_slice` /
`nr.fresh_step` gate `agent/tools.rs:96`. A two-line prompt is under 1,500 characters and every one of these is off: none of NAVARATNA ran.

## 7. Other facts the cards rely on

* Switch registry: `v3::FEATURES` (`v3/mod.rs`), read with `v3::on` / `on_in`; tests turn a switch on with `v3::forcing`. A default-on switch must
  be named in production code and in a test (`v3/promise.rs`).
* Scripted fake engine: `agent/mod.rs` `end_to_end::{raw_provider, ndjson, local_target, prompt_in}`; the step-executor tests in
  `agent/steps_tests.rs` are the pattern to copy. VCR for the engine adapters: `tests/vcr.rs` + `tests/vcr/*.jsonl`.
* The history builder is `session_bridge::history_for` (`session_bridge/mod.rs:25`); a turn's stored text is built in `sdcp/methods.rs:735-780`.
* `MEGHNA.md` was not present on disk at kickoff; its R1-R11 are taken from the rules repeated in `NAVARATNA.md` and `PADMA-PROMPTS.md`.
