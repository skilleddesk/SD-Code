# Prompt Book SDC

Oct 16, 2026 · @Mehedi Hasan

## ব্যবহারের নিয়ম ও Prompt 0

এই বইয়ে Master Plan SDC-এর ৬৬টি task-এর ইংরেজি prompt, একই ID ও ক্রমে, সাথে Prompt 0 ও NEXT (মোট ৬৮টি)। Markdown হিসেবে export করে রাখুন `sdc/docs/PROMPT-BOOK-SDC.md`। প্রথম দিন: Prompt 0, তারপর P0-0। এরপর প্রতি session-এ শুধু Prompt 0 আর NEXT; SDC নিজে পরের task খুঁজে নেবে। প্রতিটি task-এর prompt ছোট রাখা হয়েছে, কারণ সব সাধারণ নিয়ম Prompt 0-তে।

## Prompt 0 — Session bootstrap (paste at the start of every session)

```text
You are working on SDC (Skilleddesk Code), a desktop AI coding cockpit (Rust core + daemon `sdcd`, desktop UI, SDC Anywhere web remote, VPS agents). Windows, macOS and Linux are all first-class.

SOURCE OF TRUTH
- Read sdc/docs/MASTER-PLAN-SDC.md (Bengali) and sdc/docs/PROMPT-BOOK-SDC.md before any work.
- sdc/docs/PLAN-STATUS.md is the task tracker. You may change task status, never the plan itself. If the plan looks wrong, stop and report; do not improvise a new plan.

TWELVE NON-NEGOTIABLE PRINCIPLES
1. The user-selected model is the driver. Cloud/API/CLI model selected => Manual Pin: every call (planner, sub-agent, review) uses only that model; router, cascade, failover, ladder, planner-executor, bandit are OFF. Local model selected => Local Rotation: each step tries local first, then the user's ordered Rotation Chain, then the next step returns to local. Local-only when chain is empty/disabled or policy says local-only. Never change models silently.
2. Every model call goes through ModelGate. A call to a non-permitted model is a bug and must fail tests (PinViolationError).
3. Context must never be truncated silently. Count tokens before every call; if over budget, split the work. Use Ollama's native /api/chat with an explicit num_ctx, never /v1 for Ollama.
4. Compiler, LSP and tests decide pass/fail. Model confidence (entropy) is only a caution signal.
5. Edits are symbol-anchored with a content-hash check, exact or whitespace-normalized match only, fail-closed. Every step is a transaction: all or nothing.
6. Quality never drops (Q3): a token-saving technique that lowers pass rate by >1 point on a model is disabled for that model.
7. Rules live in code, not prompts.
8. Speed is a gate: single-step tasks must not get >5% slower (median/p90). Questions answerable by Code Graph/git make zero model calls.
9. Local-first. All code execution for verification runs in the Sandbox Runner. Secrets never leave the machine except inside an explicitly allowed capsule after redaction, and never from secret_paths.
10. UI extends the existing SDC UI (docs/UI-GUIDE.md): existing components, tokens, colors; every string in English and Bengali (locales/bn.ts and existing string files).
11. Every new behavior sits behind its own feature switch; switch off = exactly the old behavior, proven by a test.
12. Be honest: if something cannot be done or verified, say so; never claim success without evidence.

WORKFLOW FOR EVERY TASK
1. Read the task prompt, its dependencies and acceptance criteria. Restate the goal in 3-5 lines.
2. Inspect the existing code first; reuse before writing new code. List files you will touch.
3. Write tests first (unit + integration; special tests where listed: kill -9, fuzz, golden files, visual regression, Pin Guard, overflow).
4. Implement in small commits on a branch named task/<ID>. One task = one feature; no unrelated refactors.
5. Run the full CI locally for all three OS targets where possible (or the CI matrix), plus the relevant bench/speed checks.
6. Measure, do not guess. Report numbers with how they were measured.
7. Write docs/reports/<ID>.md: what changed, tests, measured numbers, risks, switch name, and a short Bengali summary (৫-৮ লাইন) at the top.
8. Update PLAN-STATUS.md (done or blocked with reason) and CHANGELOG.md.

STOP AND ASK when: a requirement conflicts with these principles; a destructive migration is needed; a secret or paid API call beyond the bench budget is needed; or acceptance criteria cannot be met.
```

## NEXT

```text
NEXT

1. Read sdc/docs/PLAN-STATUS.md. If it does not exist, the next task is P0-0.
2. Pick the FIRST task in plan order whose status is todo (or in-progress from an interrupted session), whose dependencies are all done, and whose phase gate is passed (a phase N task requires docs/GATES.md to show Gate N-1 = PASS).
3. If an in-progress task exists, resume it from its journal/branch instead of starting a new one.
4. Print: task ID, title, dependencies, switch name, acceptance criteria. Then load that task's prompt from sdc/docs/PROMPT-BOOK-SDC.md and follow Prompt 0's workflow exactly.
5. When finished: set status done or blocked (with reason and what is needed), link docs/reports/<ID>.md, update CHANGELOG.md.
6. If this completed the last task of a phase, run that phase's gate measurements, write the result to docs/GATES.md (PASS/FAIL with numbers). On FAIL, do not start the next phase; list which tasks must be revisited and mark them todo with a note.
7. Never skip order, never run two tasks at once, never edit MASTER-PLAN-SDC.md or PROMPT-BOOK-SDC.md.
8. End with a 3-line Bengali status: কী শেষ হলো, কী মাপা হলো, পরের task কোনটা।
```

---

## Phase 0: ভিত্তি, নিরাপত্তা ও মাপ

### P0-0 Plan Tracker

TASK P0-0 — Plan Tracker and NEXT runner. Depends: none. Switch: n/a.

Goal: make SDC able to follow the plan by itself.

Do:
- Ensure sdc/docs/MASTER-PLAN-SDC.md and sdc/docs/PROMPT-BOOK-SDC.md exist (user exports them; if missing, stop and ask).
- Create sdc/docs/PLAN-STATUS.md: one row per task (all 66 IDs P0-0..P5-5 in plan order) with columns ID, title, phase, depends_on, status (todo|in-progress|blocked|done), switch, report link, note. Derive rows by parsing the Prompt Book headings; add a script that regenerates/validates it.
- Create docs/GATES.md (one section per gate, empty) and docs/reports/.
- Implement the NEXT selection logic as code (sdc plan next): order, dependencies, phase gate, resume in-progress. Also sdc plan status and sdc plan validate (detects missing IDs, cycles, unknown deps).

Tests: selection with mixed statuses; gate FAIL blocks next phase; in-progress resumes; validator catches a missing ID.

Done when: sdc plan next prints P0-1 after this task is marked done.

### P0-1 Characterization tests and baseline

TASK P0-1 — Characterization tests and benchmark baseline. Depends: P0-0. Switch: n/a.

Goal: freeze current behavior before any change.

Do:
- Add characterization (golden) tests for the agent loop, the 5 engine adapters, sub-agents, permission gate, journal/resume, Verify, Trust Kernel ledger/policy/cost, existing token techniques per model.
- Run the existing 42-task bench on every currently connected model; store results as baseline (pass rate, tokens, cost, time, tool calls) in bench/baselines/<date>/.
- Record file-search latency (current 2.5-3.2 ms) and the known slow file-search path the user reported; add a failing perf test for it (fixed later in P1-1).

Done when: CI runs the characterization suite on 3 OS and the baseline report exists.

### P0-2 Keychain and Claude compliance

TASK P0-2 — Secrets to OS keychain; official Claude CLI only. Depends: P0-1. Switch: secrets.keychain.

Do:
- Move every API key out of plaintext files into the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service/libsecret). One-time migration with user confirmation; delete plaintext after successful read-back.
- Detect stray key files in project folders (e.g. *.txt containing key patterns) and warn the user; never upload or log them.
- Claude subscription: SDC only launches the official `claude` CLI where the user is logged in. SDC must never read, copy or proxy its OAuth token. Add a test that fails if any code path reads Claude credential files.

Tests: migration round-trip on 3 OS; no plaintext key after migration; credential-read guard.

### P0-3 Modular split

TASK P0-3 — Split large monolithic files into modules without behavior change. Depends: P0-1. Switch: n/a.

Do: create modules runtime/, sme/, precision/, rotation/, guard/ (empty facades where needed). Move code mechanically; no logic change. Keep public APIs stable.

Done when: all characterization tests pass unchanged and file sizes of the split files are reported before/after.

### P0-4 Crash-resilient journal (local + VPS)

TASK P0-4 — Journal and resume for local and VPS turns. Depends: P0-3. Switch: journal.v2.

Do: extend the existing local journal to VPS turns; journal records are append-only, fsync'd, versioned; resume after kill -9, sleep, network loss. Add a step_id field reserved for the Step Runtime.

Tests: kill -9 at 20 random points per turn type, 100/100 correct resume (local and VPS); corrupted tail record is skipped safely.

### P0-5 Data layer

TASK P0-5 — Data layer. Depends: P0-3. Switch: data.v2.

Do:
- ~/.sdc/rotation.toml (chain rows: model, enabled, allowed_steps, max_cost_per_step, max_repair_retries; globals: daily/monthly caps, ask_before_cloud, offload_limit). Schema-validated, with defaults empty.
- .sdc/policy.toml may force local-only per project; precedence documented and tested.
- .sdc/audit/sent.jsonl append-only writer (what was sent, to which model, token count, redaction summary, hash of capsule).
- ~/.sdc/cache content-addressed store keyed by BLAKE3 of content, with size cap and LRU eviction.

Tests: schema errors are reported, not crashed; precedence; cache eviction; audit log is never truncated.

### P0-6 Daemon security

TASK P0-6 — sdcd local API hardening. Depends: P0-3. Switch: daemon.secure.

Do: bind 127.0.0.1 only; per-client token; Origin and Host checks; DNS-rebinding protection; rate limiting; no CORS wildcard. SDC Anywhere keeps working through its existing relay.

Tests: requests from a foreign Origin, rebinding host, missing/old token are all rejected; existing clients still work.

### P0-7 UI guide and visual baseline

TASK P0-7 — docs/UI-GUIDE.md and UI baselines. Depends: P0-3. Switch: n/a.

Do: extract design tokens, components (ModelSelector, TrustChip, PlanCard, QuestionCard, ResearchCard, CostMeter, PolicyEditor, ProofTab, Time Machine, Workbench boards) and string files into UI-GUIDE.md. Add visual regression screenshots for every current screen (desktop + phone width) and keep axe CI green.

Done when: CI fails on any unintended pixel change of existing screens.

### P0-8 Local Runtime Guard

TASK P0-8 — Local Runtime Guard (no silent truncation). Depends: P0-3. Switch: local.runtime_guard.

Why: Ollama silently drops the start of a prompt that exceeds num_ctx (HTTP 200), and the OpenAI-compatible /v1 path ignores options so num_ctx cannot be set.

Do:
- Use Ollama native /api/chat for all local calls; always send options.num_ctx from the model profile (temporary default from /api/show until P0-9).
- Count tokens before sending (model tokenizer via /api/tokenize when available, otherwise a conservative estimator that over-counts). If prompt + max output > budget, never send: return ContextOverflow so the caller splits.
- After each call compare prompt_eval_count with the expected count; mismatch beyond tolerance => mark the result invalid and log.
- Same budget check for every cloud model using its documented limits.

Tests: a 2x-overflow prompt is never sent; a hidden sentinel at the start of a near-limit prompt is answered correctly; /v1 is not used for Ollama.

### P0-9 Model identity and capability probe

TASK P0-9 — Model identity and Capability Probe. Depends: P0-8. Switch: models.probe.

Do:
- Model key = provider + model digest (Ollama) or version id (API) + quantization + runtime version. Any change => profile becomes provisional and is re-probed.
- Probe (no user data): effective context (needle tests at 25/50/75/100% of candidate sizes), max reliable output, tool-call reliability (20 calls, incl. text-printed tool calls), JSON-schema adherence, tok/s, TTFT, VRAM in use and GPU/CPU split, logprobs support, thinking support.
- Store in ~/.sdc profiles; show class S/M/L in ModelSelector.

Tests: probe is deterministic with fixed seed; changing digest invalidates; probe never sends secrets.

### P0-10 Model-selection contract and Pin Guard

TASK P0-10 — Manual Pin, Local Rotation, Local-only; ModelGate. Depends: P0-5, P0-9. Switch: models.contract (default ON after tests).

Do:
- Central ModelGate(call) checks every model call against the active mode and pinned_model_id. Route ALL existing call sites (agent loop, sub-agents, review, research, cascade/router/failover code) through it.
- Manual Pin: disable router W6, cascade I3, failover I9, ladder, planner-executor, bandit. Provider down/rate limit => save journal and show a card: Retry / Wait / Run this task on another model (user picks).
- Explicit per-feature exceptions only from user Settings (e.g. /research final model, Duel reviewer), printed separately on the receipt.
- Mode badge next to the model button: Pinned / Local Rotation (n) / Local-only (bn + en strings).

Tests: run all bench tasks in Manual Pin and assert provider logs contain only the pinned model (3 OS); PinViolationError raised for a forced foreign call; switch off = old behavior.

### P0-11 Bench rules, hard tier and small-model bench

TASK P0-11 — Trustworthy benches. Depends: P0-9. Switch: n/a.

Do:
- Bench rules: fixed seeds, temperature per Step Profile, N>=3 runs per task, paired runs for McNemar, report noise (pass-rate spread) next to every result, --max-usd budget.
- Hard tier: tasks that the current baseline fails, including private tasks taken from the user's real projects (never published).
- Small-model bench for a local 8-9B model: long-doc QA with quotes, multi-file feature, rename across 20+ files, cut-off-prone long function, Bangla/Banglish instructions, near-context-limit prompts, no-test project edits.

Done when: bench produces a Bengali summary and machine-readable JSON; McNemar helper is tested.

### P0-12 Speed baseline and Step Trace

TASK P0-12 — Speed Gate CI and Step Trace. Depends: P0-11. Switch: trace.steps.

Do:
- Step Trace in SQLite: per call/step: task id, step id, model key, mode, tokens in/out, thinking tokens, TTFT, tok/s, wall time, verify result, cost, escalation reason.
- Speed Gate in CI: single-step task median/p90 per model vs. previous release; zero-model questions must make 0 model calls; fail the release on >5% regression.
- Small timeline view in Workbench using existing components.

Tests: gate fails on an injected 10% slowdown; trace overhead < 1 ms per step.

### P0-13 Output compressor, OS normalizer, command runner

TASK P0-13 — Tool Output Compressor, OS-Normalizer, Command Runner. Depends: P0-3. Switch: tools.compress, tools.osnorm, tools.runner.

Do:
- Compressor: keep failing test names, first/last frames of stack traces, error lines with file:line, counts; drop repeated noise. Must never drop an error line (property test).
- OS-Normalizer: models emit neutral tool calls (paths with /, abstract commands like remove_dir); Rust layer maps to the OS (paths, shell, quoting, line endings, executable names).
- Command Runner: timeout per command class, streaming output, no-output watchdog, kill tree, structured result {exit, timed_out, summary}.

Tests: golden logs from npm/cargo/pytest/go; same tool call works on 3 OS; hanging process is killed and reported.

### P0-14 Sandbox Runner and Step Transaction

TASK P0-14 — Sandbox Runner and Step Transaction. Depends: P0-13, P0-4. Switch: exec.sandbox, edit.transaction.

Do:
- Sandbox: verification code (tests, smoke, model-written scripts) cannot write outside the project/shadow dir, network off by default (allow-list per project), CPU/memory/time limits. Use the best native mechanism per OS; document the guarantees honestly per OS.
- Step Transaction: apply a step's edits in a shadow worktree; commit to the real tree only if Verify is green; otherwise roll back fully. Each commit is a Time Machine point.

Tests: escape attempts (write to home, network call) fail; partial multi-file failure leaves the repo unchanged; kill -9 mid-commit recovers.

### P0-15 Proof-of-Work badge and receipt

TASK P0-15 — Proof-of-Work badge and receipt. Depends: P0-12. Switch: ui.proof_badge.

Do: end-of-turn badge (tests passed, LSP errors, lines changed, reused functions, models per step) and a receipt listing every model used, local vs chain steps, time and cost per row. Use TrustChip/CostMeter styles, bn + en.

Tests: visual regression; receipt totals equal Step Trace totals.

### P0-16 Hardware Auto-Tuner and VRAM Arbiter

TASK P0-16 — Hardware Auto-Tuner and VRAM Arbiter. Depends: P0-9. Switch: local.autotune.

Do:
- Measure GPU/VRAM/RAM; propose OLLAMA_FLASH_ATTENTION=1 and OLLAMA_KV_CACHE_TYPE=q8_0 (server-wide env; needs Ollama restart, so ask first), the largest safe num_ctx that keeps 100% GPU, keep_alive during tasks.
- Detect CPU spill (ps + tok/s drop) and warn.
- VRAM Arbiter: only the main local model on GPU; rerankers, embeddings and classifiers are forced to CPU.
- Shown as a proposal in the Provider Hub doctor flow; never applied silently.

Tests: on an 8 GB GPU profile the proposal keeps the 9B fully on GPU; arbiter refuses a second GPU model.

### P0-17 Phase 0 release and Gate 0

TASK P0-17 — Phase 0 release and Gate 0 audit. Depends: all P0. Switch: n/a.

Do: run Gate 0: 3-OS CI green; Manual Pin foreign calls = 0; overflow test silent truncation = 0; plaintext keys = 0; baseline, speed baseline, Step Trace and both benches present. Write docs/GATES.md Gate 0 with numbers. Release nightly -> beta.

---

## Phase 1: Step Runtime ও Small Model Engine

### P1-1 Code Graph and zero-model queries

TASK P1-1 — Tree-sitter Code Graph, Merkle cache, Zero-Model Query Engine. Depends: Gate 0. Switch: graph.v1.

Do:
- Tree-sitter parsing for the languages SDC already supports; symbols, definitions, references, imports, test<->source links.
- Merkle tree of file content hashes; reindex only changed subtrees; per-file artifacts keyed by blob hash; cross-file edges recomputed when a dependency's hash changes. Branch switch must not serve stale data.
- Zero-model answers: where is X, who calls X, which tests touch file F, what changed in git. Fix the slow file search found in P0-1.

Tests: branch switch correctness; incremental reindex time; queries make 0 model calls; perf numbers in the report.

### P1-2 Step Runtime

TASK P1-2 — Step Runtime (DAG executor). Depends: P1-1, P0-14. Switch: runtime.steps.

Do:
- Task graph of steps; each step has pre-conditions (symbols/signatures it assumes) and post-conditions, checked in code.
- Execute: assemble context -> route (via ModelGate) -> patch in transaction -> verify -> commit/rollback -> journal.
- On a violated pre-condition of a later step, re-wire only the dependent subgraph (no global re-plan). Single-step tasks bypass the graph (fast path stub, finalized in P1-15).
- Reuse the existing agent loop as the step executor.

Tests: re-wire only touches dependents; resume mid-graph after kill -9; switch off = old loop.

### P1-3 Small Model Profile and budgeter

TASK P1-3 — Model classes and per-step token budgets. Depends: P0-9. Switch: sme.budget.

Do: from the probe, assign S/M/L and a per-step budget (input sections + output reserve + thinking reserve). Context Budgeter enforces section caps (system, card, memory notes, code, tool schemas, tool output). Integrates with Runtime Guard overflow handling.

Tests: no step exceeds its budget on the small-model bench.

### P1-4 Step Profile (sampling and thinking)

TASK P1-4 — Step Profiles. Depends: P1-3. Switch: sme.step_profile.

Do: per step type (plan, read, patch, format/json, debug, review, research) define temperature/top_p/top_k/presence_penalty, thinking on/off and a thinking-token cap. Start from the model vendor's documented recommendations (e.g. Qwen3.5: precise coding in thinking mode temp 0.6, presence_penalty 0; non-thinking for format steps). Profiles are per model key and must be validated on the bench before default ON.

Tests: profile applied per step (trace shows it); Q3 vs Ollama defaults reported.

### P1-5 Prompt Diet and stable prefix

TASK P1-5 — Prompt Diet, Edge Pinning, stable prefix. Depends: P1-3. Switch: sme.prompt_diet.

Do: short system prompt for S/M models; max 5 relevant tool schemas per step; goal at the start and repeated at the end; fixed ordering System + RepoMap + Working Memory header so prefix caches hit (measure cache hits for Ollama/llama.cpp and cloud APIs).

Tests: prefix byte-identical across steps of one task; Q3 per model.

### P1-6 Constrained output and Tool-call Rescue

TASK P1-6 — Grammar/JSON-schema output and Tool-call Rescue. Depends: P1-5. Switch: sme.constrained, sme.toolcall_rescue.

Do: use Ollama structured outputs / schema for notes and tool calls; repair layer for near-valid JSON; detect tool calls printed as text (e.g. <tool_call>...</tool_call> blocks), parse, validate against schema, and execute; record runtime versions with known issues in the profile.

Tests: golden corpus of broken outputs; rescued calls are schema-valid; invalid ones are rejected, not guessed.

### P1-7 Working Memory

TASK P1-7 — External Working Memory. Depends: P1-2. Switch: sme.memory.

Do: SQLite ledger per task: intent card, plan, verified notes (with source quote + location), decisions, handover notes. Context Assembler pulls only notes linked to the current step. Right-panel tab (existing styles, bn + en) to view/edit/pin notes.

Tests: long 10-step task keeps intent intact; memory survives restart.

### P1-8 Retrieval Cascade and Chunked Reader

TASK P1-8 — Retrieval Cascade and Chunked Reader. Depends: P1-1, P0-16. Switch: sme.retrieval.

Do:
- Stage 1 (zero-model): Code Graph, BM25, headings, file names.
- Stage 2 (optional): CPU cross-encoder reranker (ONNX); multilingual model for Bangla docs; must run on CPU (VRAM Arbiter).
- Stage 3: the step model reads only top-k chunks.
- Chunk by AST nodes and Markdown headings; skim-then-deep; summaries cached by content hash; live progress (reading 12/40).

Tests: recall@k on the bench; stage 2 enabled only if it improves recall and passes Speed Gate.

### P1-9 Source Quote Verifier

TASK P1-9 — Source Quote Verifier. Depends: P1-7. Switch: sme.quote_verify.

Do: every note must carry quotes with file + range; code checks the quote exists (exact or whitespace-normalized). Failing notes are rejected and that chunk is re-read once; persistent failure is reported.

Tests: fabricated quotes = 0 accepted on the bench.

### P1-10 Task Decomposer

TASK P1-10 — Task Decomposer, prefetch, as-needed decomposition. Depends: P1-2, P1-8. Switch: runtime.decompose.

Do: split tasks into steps of 1-2 functions using the Code Graph; code (not the model) selects files; prefetch next step's files/AST; when a step fails after repairs, try one level of further decomposition of that step only.

Tests: steps fit budgets; as-needed decomposition improves pass rate on the hard tier or is left OFF.

### P1-11 Symbol-Anchored Patcher and Cut-off Guard

TASK P1-11 — Symbol-Anchored Patcher and Cut-off Guard. Depends: P1-1, P0-14. Switch: edit.symbol_patch.

Do:
- Edit target = fully qualified symbol path (file::Class::method) + hash of the old body. Both must match. Exact or whitespace-normalized match only; no fuzzy matching; on mismatch fail closed and re-read.
- Duplicate names (overloads, same name in different scopes) must be disambiguated or rejected.
- Small functions: whole-symbol rewrite; larger: search/replace inside the symbol.
- Cut-off Guard: detect truncated output, ask for continuation from the last valid AST node, stitch, re-parse.

Tests: fuzz with duplicate names and drifted whitespace => 0 silent misapplies; truncated outputs repaired or rejected.

### P1-12 LSP Bridge

TASK P1-12 — LSP Bridge and guided repair. Depends: P1-1. Switch: verify.lsp.

Do: language-server pool (e.g. tsserver/typescript-language-server, pyright/ty, rust-analyzer, gopls), started lazily and kept warm per project; diagnostics for changed files; for each error, attach the definition/signature of involved symbols to the repair prompt; fallback to compiler/linter when no server exists.

Tests: repair success with LSP context vs. without on the bench; server crash is recovered.

### P1-13 Confidence Signals

TASK P1-13 — Confidence Signals and loop detector. Depends: P1-2. Switch: sme.confidence.

Do: when logprobs are available (Ollama >= 0.12, some APIs), compute top-k token entropy on decision tokens; always compute structural signals (repeated identical errors, oscillating edits, schema breaks, tool loops). Signals may only raise caution: deeper verify, earlier decomposition, earlier escalation in Local Rotation. They may never mark a step as passed.

Tests: report AUROC of the signal vs. step failure on the bench; loop detector stops loops within N iterations.

### P1-14 Bangla/Banglish Spec Brief

TASK P1-14 — Bangla/Banglish to English Spec Brief. Depends: P1-5. Switch: sme.spec_brief.

Do: Intent Engine turns a Bangla/Banglish instruction into a short precise English technical spec for S/M models; the user sees a confirmation in their own language; keep a glossary per project.

Tests: Bangla bench tasks pass rate with vs. without brief; confirmation strings in bn + en.

### P1-15 Fast path and Gate 1

TASK P1-15 — Single-step fast path and Speed Gate; Gate 1. Depends: all P1. Switch: runtime.fastpath.

Do: classify requests; single-step edits/questions skip planner and SME; zero-model questions answered from graph/git. Run Gate 1: same 9B digest, Local-only, SME vs naive, paired runs, McNemar p < 0.05; cut-off, fake quotes and silent mis-patches = 0; speed within 5%. Write docs/GATES.md.

---

## Phase 2: Local Rotation

### P2-1 Rotation Policy Engine

TASK P2-1 — Local Rotation policy. Depends: Gate 1. Switch: rotation.policy.

Do: decide per step when to escalate. Pre-step: step does not fit budget after decomposition; Step Trace shows this step type succeeds <30% for this model key (minimum sample size). Post-verify: still red after 2 LSP-guided repairs and one as-needed decomposition; structural loop/confidence signals. Never escalate for environment problems (Local Watchdog list) or secret_paths. Manual Pin and Local-only can never reach this code (ModelGate test).

Tests: decision table tests; every escalation has a logged reason.

### P2-2 Rotation Chain Settings UI

TASK P2-2 — Settings -> Agent -> Local Rotation. Depends: P2-1, P0-5. Switch: ui.rotation_chain.

Do: list with up/down buttons, drag on desktop, enable toggle, delete; per-row model, allowed steps, max cost per step, max repair retries; global daily/monthly caps, ask-before-cloud, offload limit; privacy local-only. Built from PolicyEditor patterns; phone layout; bn + en; axe + visual regression.

Tests: order persists to rotation.toml exactly; keyboard reordering works.

### P2-3 Capsule Minimizer, Redactor, audit

TASK P2-3 — Capsule. Depends: P2-1. Switch: rotation.capsule.

Do: capsule = intent card + step goal + target symbol code + verified notes + exact verify error; dedupe notes; secret redaction (keys, tokens, .env values, emails if policy says); refuse secret_paths; preview card (summary, size, estimated cost) in Ask/Pro; append to .sdc/audit/sent.jsonl; pre-warm capsule after first local repair failure.

Tests: secret corpus never appears in capsules; audit entry per send; capsule fits target context.

### P2-4 Chain Escalation Runner

TASK P2-4 — Escalate in chain order. Depends: P2-3. Switch: rotation.runner.

Do: send to the first enabled and eligible row (credentials/login present, step type allowed, within cost caps, capsule fits); same Verify gate; per-row repair retries; on failure next row with capsule + last failure; skip unavailable rows at no cost with reason; CLI rows (Claude Code, Codex CLI) run in a separate worktree and return a diff; after success, next step returns to local. All chain rows fail => honest stop with summary.

Tests: order never violated (property test); return-to-local 100%; CLI diff goes through the same Verify.

### P2-5 Handover Bridge Note

TASK P2-5 — Handover Note. Depends: P2-4, P1-9. Switch: rotation.handover.

Do: after a chain step succeeds, code derives "what changed" from the diff (symbols, new signatures, new files); the chain model adds <=80 tokens of "why" and "constraints for next steps"; quotes verified; stored in Working Memory and pinned at the top of the next local step.

Tests: next-step break rate after escalation with vs. without the note.

### P2-6 Planner-Executor

TASK P2-6 — Planner-Executor (Local Rotation only). Depends: P2-4. Switch: rotation.planner_exec.

Do: for tasks the decomposer marks as design-heavy, ask chain #1 for the plan/graph only; all code-writing steps run local. Requires user permission in Ask/Pro. Blocked in Manual Pin/Local-only by ModelGate.

Tests: cost and pass rate vs. plain Local Rotation on the bench; keep OFF if Q3 fails.

### P2-7 Chain-to-local learning and Failure Taxonomy

TASK P2-7 — Recipes from chain solutions; Failure Taxonomy. Depends: P2-5. Switch: learn.chain_recipes.

Do: turn verified chain solutions into recipes via knowledge_core (proof gate, rollback); classify failures (type error, wrong file, missing context, format, logic, environment) and map each to a targeted repair strategy.

Tests: on replayed repeat tasks, chain usage drops without pass-rate loss; poisoned recipe attempts blocked.

### P2-8 Rotation safeguards and Gate 2

TASK P2-8 — Local Watchdog, Offload Limiter, Chain Health, proposed chain; Gate 2. Depends: all P2. Switch: rotation.safeguards.

Do: Watchdog stops (never offloads) on local server down, model unloaded, CPU spill, missing toolchain, no network; Offload Limiter asks after 5 cloud steps or 50% of steps; Chain Health pre-check of keys/logins/caps with zero model calls; first-time proposed chain card from bench results, never auto-enabled. Run Gate 2 and write docs/GATES.md.

---

## Phase 3: Precision ও Cursor-মানের UX

### P3-1 Reuse Finder

TASK P3-1 — Reuse Finder. Depends: Gate 2. Switch: precision.reuse.

Do: before writing a new function, search Code Graph, installed packages and stdlib for near matches (name, signature, doc, embedding when P3-12 exists); give the model a compact list ("formatCurrency() exists in utils/money.ts"). Count reused functions on the badge.

Tests: duplicate-function rate on the bench drops; Q3 per model.

### P3-2 Diff Budget

TASK P3-2 — Diff Budget. Depends: P3-1. Switch: precision.diff_budget.

Do: expected change size per task class (learned from accepted diffs); oversized diffs are rejected with a reason and a request to shrink; user can override per task.

Tests: needless rewrites blocked; legitimate refactors pass with override.

### P3-3 Hallucination and Slopsquatting Guard

TASK P3-3 — Import and package existence guard. Depends: P1-1. Switch: precision.hallucination.

Do: before any install or run, check every new import/package against manifests, lockfiles, local disk and (with permission) the registry; flag look-alike names; never auto-install unknown packages.

Tests: corpus of fake and typosquat packages = 0 accepted.

### P3-4 Test Impact, Characterization, Smoke

TASK P3-4 — Test Impact Runner with characterization and smoke for test-less code. Depends: P1-1, P0-14. Switch: verify.test_impact.

Do: map changed symbols to tests via the graph; when uncertain, run more, not fewer. If no tests cover a changed function, capture its current behavior (golden outputs for sample inputs) BEFORE the edit, compare after; then run an import/run smoke. All inside the sandbox.

Tests: missed-test rate = 0 on a mutation suite; characterization catches injected behavior changes.

### P3-5 Quality Floor and AST Duplicates

TASK P3-5 — Quality Floor. Depends: P1-1. Switch: precision.quality_floor.

Do: fail on empty catch blocks, deleted/skipped tests, removed error handling or input validation, AST-duplicate functions, new unused code. Rules configurable per project.

Tests: golden violations detected; no false positive on the baseline repos.

### P3-6 Ripple Check

TASK P3-6 — Ripple Check. Depends: P1-12. Switch: precision.ripple.

Do: on signature/behavior change, list all affected call sites (graph + LSP references) and create one small step per site; verify each.

Tests: rename across 20+ files completes with 0 broken references.

### P3-7 Spec-to-Test

TASK P3-7 — Spec-to-Test. Depends: P3-4. Switch: precision.spec_tests.

Do: from the Intent Contract acceptance conditions, generate tests before implementation; user can view/approve them in Diff Review.

Tests: generated tests fail before and pass after on bench feature tasks.

### P3-8 Q3 profiling for all models

TASK P3-8 — Per-model Q3 profiling of token techniques. Depends: P0-11. Switch: n/a.

Do: run W4, W5, L0-L3, stable_prefix, ledger_diff, skeleton, interlingua/spec brief, compressor on every connected model key (>= 4 providers + 1 local 9B); enable only where Q3 passes; store provisional/proven status.

Done when: profile table per model is in the report with paired-run statistics.

### P3-9 Diff Review UI

TASK P3-9 — Diff Review. Depends: P0-7. Switch: ui.diff_review.

Do: side-by-side diff in the right panel; per-hunk Approve / Reject / Ask AI to change; verify results, risk marks and which model wrote each hunk; phone layout; bn + en.

Tests: visual regression, axe, keyboard-only flow.

### P3-10 @ Context Picker

TASK P3-10 — @ picker. Depends: P1-1. Switch: ui.at_picker.

Do: @file, @folder, @symbol, @docs, @git, @web, @server, @project with token-size preview; selections go straight into the step context (no search needed).

Tests: token counts match the budgeter; works on phone.

### P3-11 Select & Instruct

TASK P3-11 — Select & Instruct. Depends: P3-9. Switch: ui.select_instruct.

Do: select lines in Preview, type a short instruction in a QuestionCard-style input; becomes a single-symbol step via the fast path.

Tests: scope limited to the selection; result shown in Diff Review.

### P3-12 Local Semantic Index

TASK P3-12 — Local semantic index on CPU. Depends: P1-8. Switch: graph.semantic.

Do: small local embedding model on CPU, chunk embeddings cached by content hash; hybrid with BM25 + Code Graph; size cap.

Tests: recall@k improvement on the bench; no VRAM use; Speed Gate passes.

### P3-13 Scoped Rules, Hooks, rule import

TASK P3-13 — .sdc/rules and .sdc/hooks.toml. Depends: P1-5. Switch: rules.scoped.

Do: path-scoped rules injected only when matching (compact summary for S/M models); hooks before/after steps; one-click import of .cursorrules, CLAUDE.md, AGENTS.md; memory additions need user approval.

Tests: only matching rules enter context; imported rules round-trip.

### P3-14 Verified MCP Installer

TASK P3-14 — One-click MCP installer. Depends: P0-6, P0-2. Switch: mcp.installer.

Do: curated list with source verification; secrets to keychain; per-server permission scopes; MCP outputs treated as untrusted data.

Tests: install/uninstall on 3 OS; injection strings in MCP output cannot change mode or chain.

### P3-15 Local GPU Scheduler and Gate 3

TASK P3-15 — Local GPU Scheduler for many projects; Gate 3. Depends: all P3. Switch: local.scheduler.

Do: one queue for the local GPU across projects and VPS-triggered work; priorities (interactive > background), batching of small read steps, keep the same model loaded, no model-swap thrash; Workbench shows who is waiting. Run Gate 3 (Q3 for every enabled technique; missed tests = 0; UI visual/axe pass; 3 concurrent projects lose no steps) and write docs/GATES.md.

---

## Phase 4: শেখা ও ইকোসিস্টেম

### P4-1 Replay Lab

TASK P4-1 — Replay Lab. Depends: Gate 3. Switch: learn.replay.

Do: re-run recorded tasks (from journal + Step Trace, with user consent, inside the sandbox) on a new technique, profile or model; paired comparison of pass rate, cost, time; budget-capped.

Tests: replay is deterministic within measured noise; no network side effects.

### P4-2 Smart Router

TASK P4-2 — Smart Router (Local Rotation only). Depends: P4-1. Switch: rotation.smart_router.

Do: learn from replay data which step types should skip local and go directly to chain #1 because local almost always fails there. It may never reorder the chain. Inactive in Manual Pin/Local-only (ModelGate test).

Tests: lower cost at equal pass rate on replay; chain order unchanged (property test).

### P4-3 Night Learning, Drift Monitor, weekly report

TASK P4-3 — Night Learning and weekly Bengali report. Depends: P4-1. Switch: learn.night.

Do: optional nightly replay within budget to re-verify recipes; weekly drift check per model key (pass rate, cost, speed); auto-disable techniques that fail Q3; weekly Bengali report in Workbench Inbox with one-click proposals (e.g. chain order change) that the user accepts or ignores.

Tests: injected drift disables the technique; report renders in bn.

### P4-4 SDC as MCP Server

TASK P4-4 — SDC MCP server. Depends: P0-6. Switch: mcp.server.

Do: expose Verify, Code Graph queries, Proof, Safe Deploy dry-run as MCP tools via sdcd with per-client tokens and scopes; outputs never include secrets.

Tests: penetration tests (unauthenticated, wrong origin, scope escalation) all fail.

### P4-5 VS Code Companion

TASK P4-5 — Light VS Code companion. Depends: P4-4. Switch: ext.vscode.

Do: extension only shows diffs, step status and proof badges and sends commands; all work runs in sdcd. Uses the same auth.

Tests: security tests pass; extension works with daemon on 3 OS. Run Gate 4 and write docs/GATES.md.

---

## Phase 5: বহু-মেশিন ও অটোমেশন

### P5-1 Continuity Sync

TASK P5-1 — Continuity Sync. Depends: Gate 4. Switch: sync.continuity.

Do: sync journal, Working Memory, recipes, settings (not caches, not secrets) between the user's machines via the existing Cloudflare relay with end-to-end encryption; keys only on user devices; resume an unfinished task on another PC.

Tests: relay sees only ciphertext; conflict resolution; resume across OS.

### P5-2 DevOps Dry-run and Playbook Verifier

TASK P5-2 — Dry-run and Playbook Verifier. Depends: P5-1. Switch: ops.dryrun.

Do: every VPS/DevOps playbook runs as dry-run first with a diff of intended changes; verify on a real Docker fleet test bed; fleet (I7) output dedupe measured on real servers.

Tests: destructive commands require explicit approval; dry-run matches real run on the test bed.

### P5-3 Background Agent on the user's VPS

TASK P5-3 — Background agent on own VPS. Depends: P5-1. Switch: agent.background.

Do: run long tasks on the user's VPS (any git or folder), same Step Runtime and Verify, phone notifications and permission cards via SDC Anywhere (sdc.skilleddesk.com), journaled.

Tests: VPS reboot mid-task resumes; permissions time out per policy.

### P5-4 Self-hosted PR Review Bot

TASK P5-4 — PR Review Bot. Depends: P5-3. Switch: agent.pr_review.

Do: Verify gate + review on PRs using the user's chosen model and keys; posts findings with proof; never pushes without approval.

Tests: review on sample PRs; secrets never in comments.

### P5-5 GitHub Issues, Telegram, Slack

TASK P5-5 — Integrations. Depends: P5-3. Switch: integrations.messaging.

Do: start tasks from GitHub Issues, Telegram, Slack threads; reply with an Intent Contract for approval before work; message content is untrusted data and cannot change mode, chain or permissions.

Tests: injection strings in messages are inert; end-to-end task from an issue to a verified PR draft.
