# 3.1 work log - resume from here

Started 2026-10-18 from 3.0.3 (the owner's brief: Phases 0-5). A new session reads this file and `git log`, then continues at the first
line that is not DONE. Evidence for every DONE line is in the commit named, its tests, or `docs/VOICE.md`.

Legend: DONE (verified), PARTIAL (what works / what is missing), OWNER (needs the owner), TODO.

## Phase 0 - security first
- 0.1 DONE - the six secret files are in `~/Documents/sdc-secrets` (mode 700), none left in the work folder. OWNER: rotate every key.
- 0.2 DONE - `.sdc/policy.toml` (test: `own_policy_parses...`).
- 0.3 DONE - content-based guard `trust/secretguard.rs`, wired in `read_file`, grep/command output; tests with fake secrets.
- 0.4 DONE - first-open sweep `trust/sweep.rs`, `secrets.sweep` / `secrets.move`, modal.
- 0.5 DONE - (a) `edit.transaction` marked INACTIVE (`v3::INACTIVE`, shown in Advanced); (b) `knowledge_core::nightly` runs from the scheduler clock (`nightly_if_due`); (c) Cargo.lock 3.0.3 + `--locked` in CI (commit 93cbb0e).
- 0.6 DONE - `v3/promise.rs` CI check (production reference + a test for every default-on switch).
- 0.7 DONE - the "8 extra methods" were 8 plain string match arms, not methods; `protocol/check.mjs` now counts dotted names and requires equality.
- 0.8 DONE - one resolver (`paths.rs`: data_dir, tools_dir, backup_dir); a test fails if any other file asks `dirs::` for a folder; docs corrected (macOS root is `~/Library/Application Support/sdc`).
- 0.9 DONE - `tokens.rs` + `app/src/lib/tokens.ts`, shared fixtures `protocol/token-fixtures.json` measured with tiktoken; four estimators replaced.
- 0.10 DONE - Builder/Reviewer; no Reviewer = review off (`guard/reviewer.rs`, Verify tab).
- 0.11 DONE - old installers, duplicates archived to `~/Documents/sdc-archive`; ARCHIVED notices; tracker cleaned.

## Phase 1 - voice
- 1.1 DONE (root cause proven, see `docs/VOICE.md`). 1.2 DONE (research + measurements). 1.3 DONE (chain, download with sha256, language selector, honest errors).
- 1.4 PARTIAL - local whisper measured on 4 languages with TTS clips; cloud transcribers not measured (no Groq/Google key here). OWNER-TO-RUN: `sdcd voice FILE --no-local`.
- 1.1 end-to-end on a real microphone: OWNER (macOS prompt cannot be clicked from here).

## Phase 2 - local runtimes
- 2.1-2.3 DONE (`providers/runtimes.rs`, `local.scan`, rescan on focus/hub). 2.4 DONE (`sme/tuner.rs`). 2.5 DONE with mocks; live Ollama on the Windows PC is OWNER-TO-RUN.

## Phase 3 - Agents
- 3.1-3.5 DONE in code (Agents tab, rotation.get/set, chain-aware ModelGate, Escalate with capsule + audit). Live escalation to a paid model: OWNER-TO-VERIFY.
- 3.6 PARTIAL - gate-level contract tests (zero calls outside the chain), UI string/logic tests, axe 0 violations on the Agents tab (`SDC_A11Y_SCREEN=agents`). No full driven-turn test for the two local modes.

## Phase 4 - product
- 4.4 DONE (presets, Simple/Pro, Advanced). 4.1-4.3, 4.5-4.12: see below as they are finished.
- 4.1 DONE (optional signed updater + Windows signing in `release.yml`, secret-gated; OWNER: certificates). In-app updater UI: not built.
- 4.2 DONE (opt-in telemetry, counts only, no server). 4.3 DONE (onboarding timing test). 4.5 DONE (Ship Safe). 4.6/4.7 DONE (Time Machine on rail, cost by source).
- 4.8 DONE (morning brief + weekly client report). 4.9 DONE (`sdcd mcp`). 4.10 DONE (Windows Job Object containment; type-checked for the Windows target, runtime test OWNER-TO-RUN on Windows).
- 4.12 DONE for the task set (40 validated small tasks, Gate 1 statistics plan). The Gate 1 run itself is NOT done: it needs a live 9B model - OWNER-TO-RUN (`sdcd bench run --models ollama:qwen3.5:9b --tier small --runs 3`).
- 4.11 (Small Model Engine) - switch / evidence:
  - P1-1 `graph.v1`, P1-2 `runtime.steps`, P1-11 `edit.symbol_patch` (off by default), P1-12 `verify.lsp` (off by default): DONE with tests.
  - P1-3 `sme.budget`, P1-5 `sme.prompt_diet`: DONE, on. P1-4 `sme.step_profile`: DONE, off by default (not measured on the bench).
  - P1-6 `sme.toolcall_rescue`: DONE (corpus of 16 printed shapes, schema validation). `sme.constrained` (grammar-constrained output): NOT built.
  - P1-7 `sme.memory` working memory, P1-8 `sme.retrieval` (ranked_search tool), P1-9 `sme.quote_verify`, P1-10 `runtime.decompose` (only the "step names more than two files" check is wired into the plan tool; the planner does not yet call `breakdown`), P1-13 `sme.confidence`, P1-14 `sme.spec_brief`: DONE with unit tests; wiring as stated in the code.
  - P1-15 `runtime.fastpath`: DONE for what it skips (spec restatement); Gate 1 itself: NOT RUN (owner).
  - P3-3 `trust.slopguard` (install commands checked against npm / PyPI / crates.io; unknown name refused; unreachable registry only a note): DONE.
  - P3-9 Diff Review (per-file, per-hunk, ask / ask-to-undo prefill the prompt), P3-11 Select & Instruct (selection in a file -> prompt with `@path:line`): DONE (pure logic tested; the buttons are not covered by a browser test). P3-10 `@` picker: already existed in `PromptArea`.
  - Not done in 4.11: the loop detector beyond the existing one in `trust/cost.rs` (5th identical call), an automated confidence AUROC measurement on real runs.
