# SDC-v3 — Progress

Source of truth for resuming work on SDC-PLAN-v3. A new session reads this file first.
Status values: `todo` · `in-progress` · `blocked` · `done` · `killed` (with data) · `deferred`.
Branch naming: `sdc-v3/<ID>`. Numbers come only from `bench/results/` (never typed by hand).

- Current phase: **1 — Measure (W1)**. GATE 0 passed ("continue", 2026-10-14); owner then asked to complete the
  whole project without stopping at gates (2026-10-14) and to lift SDC AI's step limits (done, see "Owner decisions").
- Baseline: tag `sdc-baseline-v1` = `274ec6a` (= `v1.0.0`), binary `bench/bin/v1/sdcd` (not committed)
- Assumption register: `docs/plan/ASSUMPTIONS.md`
- Next action: paid baseline runs need the owner's model list + keys (W1 harness is ready and validated); meanwhile Phase 2 (R1 journal).

## Owner decisions

| Date | Decision | Effect |
|---|---|---|
| 2026-10-14 | "continue" at Gate 0 without answering the open questions | recommended defaults taken: bench budgets smoke $2 / medium $40 / full $250 per run; knowledge sync off; single-user account server; weekly code proposals |
| 2026-10-14 | "SDC AI needs no limit" | removed the sub-agent 24-step cap and the `/research` step cap; the main turn already had none. Kept: budget stop (a budget the person sets), runaway-loop guard, content-filter retry count, tool-output size caps (`RESULT_CAP`, `PAGE_CAP`) - see gate report |
| 2026-10-14 | "complete the whole project" | gates are reported but not waited on; paid runs still need models/keys |

## Phase 0 — Preparation

| ID | Item | Status | Branch | Numbers / evidence | Next action |
|---|---|---|---|---|---|
| P0.1 | Tag HEAD `sdc-baseline-v1` | done | — | tag on `274ec6a` (local, not pushed) | push the tag after owner OK |
| P0.2 | Build v1 into `bench/bin/v1/` | done | — | macOS arm64 release build from the tag; `cargo test` 738 pass / 0 fail / 6 ignored | Windows/Linux v1 builds in CI (Phase 1) |
| P0.3 | Verify A-01 (code facts) | done | sdc-v3/P0 | see ASSUMPTIONS A-01 | — |
| P0.4 | Existing-capability check | done | sdc-v3/P0 | gate-0-bn.md §2 | — |
| P0.5 | U0 uniqueness audit | done | sdc-v3/P0 | `docs/reports/U0-uniqueness-bn.md` | re-run in Phase 7 |
| P0.6 | Open decisions to owner | done | — | defaults taken (see Owner decisions) | benchmark model IDs + keys still open |
| GATE 0 | Gate report | done | sdc-v3/P0 | `docs/reports/gate-0-bn.md` | — |

## Phase 1 — Measure

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W1 | `sdcd bench` (fixtures, 40 tasks, runner, ledger, search bench, competitors, tiers, `--compare`, `--report-bn`) | in-progress | sdc-v3/W1 | harness done + validated (42/42 tasks fail untouched, pass with reference; mock end-to-end run OK; ledger = provider usage exactly, request byte-identical); search baseline `results/2026-10-14-search-v1` | paid baseline runs (needs models + keys); fleet fixture with I7; Windows/Linux v1 binaries from CI |
| GATE 1 | A-02, A-03, A-09, A-10, A-14 measured | todo | — | — | — |

## Phase 2 — Durable foundation

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| R1 | Journal (`job_events` intent/result, SQLite WAL) | done | sdc-v3/R1 | `v3/journal.rs`; start/model/intent/result/messages/end, idempotency key, output hash; switch `v3.journal` (on) | remote (VPS) turns are not journaled yet |
| R2 | Resume on start | done (local) | sdc-v3/R1 | A-19 local: 100 SIGKILLs → 100/100 correct, 85 resumed from the journal, 0 damaged files, 0 duplicated commands (`bench/results/2026-10-14-resume`); reads re-run, writes reconciled with the folder, commands reported as outcome-unknown | remote-command check on a VPS (with R4) |
| R3 | Offline queue (backoff + jitter) | done | sdc-v3/W9 | after the 8 normal retries, a step with no network waits indefinitely (30 s doubling to 5 min, ±20% jitter), still stoppable; provider errors (5xx/429/4xx) are not waited out; switch `v3.offline` (on). Search, index, git and local commands never needed the network | an end-to-end offline→online test needs injectable waits |
| R5 | Graceful shutdown on sleep/shutdown | partial | sdc-v3/R1 | SIGTERM/SIGHUP/Ctrl-C: children stopped, WAL checkpointed, running turns resume next start | OS sleep notifications (Windows/macOS power events) |
| W7a | Checkpoints (hidden git refs / CAS blobs, undo turn, timeline, VPS) | todo | sdc-v3/W7 | — | — |
| W9 | Security basics (OS key storage + migration, rate limiter, untrusted tagging, secret redaction, SQLite integrity + backup) | partial | sdc-v3/W9 | secret redaction (token shapes + named high-entropy assignments → ‹secret:N›, restored on run/write, vault kept in the journal for resume) - test: a fake key never reaches the provider; untrusted marking of web/browser/MCP text; per-host token bucket (120/min, burst 20); quick_check before the daily copy (daily backup already existed); switches `v3.redact`/`v3.untrusted`/`v3.rate_limit` (on) | Linux Secret Service key storage (keys are 0600 files there today); Windows DPAPI/macOS Keychain already used by the packaged app |
| GATE 2 | A-19 local part | todo | — | — | — |

## Phase 3 — Speed and intelligence

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W3 | Search module `sdcd/src/search/` | done | sdc-v3/W3 | 52,501 files, p50: filename 2.5-3.2 ms (v1 229-234), text 2.0-6.6 ms (v1 227-1204 incl. a no-match full scan); first call 150 ms (v1 229-2815); peak RSS 58 MB (`results/2026-10-14-search-v1` vs `-v3`); `ignore` parallel walk + .gitignore, notify-patched index, byte prefilter, trigram Bloom content index; switch `v3.search` (on) | Windows/Linux numbers in CI; `nucleo` fuzzy names + UI debounce; same engine via `sdcd search` on a VPS |
| W4 | Code intelligence (tree-sitter, symbols, repo map) | todo | sdc-v3/W4 | — | — |
| W5 | Cache layout + tool pruning | built, off (awaits Q3) | sdc-v3/W5 | `stable_prefix`: the date leaves the system prompt for the turn's message; `tool_pruning`: intent tool sets (core + web/browser/process/remember by the request's words), a called tool joins the list; test proves both change only what they say | paid bench comparison per model (Q3); Gemini context caching; Anthropic 4-breakpoint layout |
| I1 | Context Ledger | built, off (awaits Q3) | sdc-v3/W5 | `ledger_diff`: a changed whole-file re-read goes back as a unified diff (≤60% of the file), cleared on fold; v1's unchanged-read stub kept | `recall(Lk)` tool; refs on stubs |
| I2 | Skeleton-first reads | built, off (awaits Q3) | sdc-v3/W5 | `skeleton`: files >200 lines read whole answer an outline (imports summed, declarations with line numbers; 12 language families) when it saves ≥40% | tree-sitter symbols (W4); expand by symbol |
| I10 | Token Receipt | done (footer) | sdc-v3/W5 | every agent turn's footer: input by part (apportioned to the provider's count) and tokens saved per technique (unchanged re-reads, diffs, tool pruning); switch `v3.receipt` (on) | monthly "saved $X"; dev-only shadow baseline; a UI card |
| Q2 | Model profiles (generated) | done (mechanism) | sdc-v3/W5 | `protocol/model-profiles.json` written only by `sdcd bench profiles` from Q3 verdicts; per-turn switches settle setting → profile → default; unknown models: no token technique | first real profile needs the paid comparisons; best edit format per model |
| L0 | Memory | todo | sdc-v3/L0 | — | — |
| L1 | Lessons | todo | sdc-v3/L1 | — | — |
| GATE 3 | A-04–A-08, A-11; input −35% vs v1; Q3 on 5 models | todo | — | — | — |

## Phase 4 — Quality and cost

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W6 | Router (main + fast model, estimateCost wiring) | todo | sdc-v3/W6 | — | — |
| W7b | Verify loop + `/plan` | todo | sdc-v3/W7 | — | — |
| I3 | Draft–Verify cascade | todo | sdc-v3/I3 | — | — |
| I4 | Recipe Memory (= L2) | todo | sdc-v3/I4 | — | — |
| I5 | Budget Contract | done | sdc-v3/I5 | `@budget 40k` / `@budget $0.10`: opening note, converge at 80%, stop at 100% with the work kept (end-to-end test); switch `v3.budget` (on) | a live gauge in the UI |
| I9 | State Capsule failover | done | sdc-v3/I5 | `v3.failoverOrder` (`provider:model, …`): a provider that fails for good hands the task to the next, as a ≤6,000-char capsule (goal, open plan steps, changed files, last 12 steps, last words); fault-injection test (401 → next model finishes) | periodic capsules every few steps (today built at the failure); cross-provider journal resume |
| Q4 | Runtime quality guard (SPRT) | todo | sdc-v3/Q4 | — | — |
| Q5 | Lift weak models | todo | sdc-v3/Q5 | — | — |
| L2 | Recipes (see I4) | todo | sdc-v3/I4 | — | — |
| L3 | External knowledge | todo | sdc-v3/L3 | — | — |
| GATE 4 | A-12, A-13; Q3 on all models | todo | — | — | — |

## Phase 5 — Cross-device and security

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| X1 | Login (passkeys, magic link, TOTP) | todo | sdc-v3/X1 | — | — |
| X2 | Device enrolment + recovery key | todo | sdc-v3/X2 | — | — |
| X3 | E2E sync | todo | sdc-v3/X3 | — | — |
| X4 | Handoff (leases + fencing) | todo | sdc-v3/X4 | — | — |
| X5 | Hardening (pinning, revoke/wipe, signing, notarization, dep scan) | todo | sdc-v3/X5 | — | — |
| X6 | Phone browser | todo | sdc-v3/X6 | — | — |
| R4 | VPS Runner | todo | sdc-v3/R4 | — | — |
| G36 | Data export / delete / retention | todo | sdc-v3/G36 | — | — |
| GATE 5 | A-20, A-21, ASVS L2 | todo | — | — | — |

## Phase 6 — New capabilities

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W8 | `/research` | todo | sdc-v3/W8 | — | — |
| I6 | Privacy Split Router (cloud-only part) | done (cloud part) | sdc-v3/I6 | `secret_paths` in `.sdc/policy.toml`: reads refused, grep/search lines dropped, commands naming a secret file have output withheld; `.sdc/audit/sent.jsonl` records files sent per provider/model; canary end-to-end test (read + grep + cat) → 0 leaks; switch `v3.privacy` (on) | 100-task canary run on the bench; embeddings (none exist yet); local summariser deferred (D-1) |
| I7 | Fleet Fan-out | built (switch off by default) | sdc-v3/I7 | `fleet_run` tool: parallel SSH, outputs normalised (times, PIDs, ids, IPs, durations, host names) and clustered, majority sample + diffs; asks before non-read-only commands; canary-first; synthetic A-16: 20 servers −80.4%, 0 hidden diffs (`results/2026-10-14-fleet-synthetic`); switch `v3.fleet` (off: it adds a tool) | real Docker 5/10/20 fleet; `@fleet` chat syntax |
| I8 | Interlingua | todo | sdc-v3/I8 | — | — |
| I11 | Self-Improving Knowledge Core (umbrella over L0–L5) | todo | sdc-v3/I11 | — | — |
| I12 | (definition to confirm with owner — not described in the master prompt) | blocked | — | — | owner defines |
| PAR | Parallel agents in worktrees + Agents board | todo | sdc-v3/parallel | — | — |
| AUTO | Automations + Inbox | todo | sdc-v3/automations | — | — |
| L4 | Contextual-bandit router | todo | sdc-v3/L4 | — | — |
| L5 | Prompt / tool-description optimisation | todo | sdc-v3/L5 | — | — |
| G34 | UI end-to-end tests on 3 OS | todo | sdc-v3/G34 | — | — |
| G35 | Opt-in privacy-safe telemetry / crash reports | todo | sdc-v3/G35 | — | — |
| G37 | Respect site terms (research) | todo | sdc-v3/W8 | — | — |
| G39 | Accessibility + first-run onboarding | todo | sdc-v3/G39 | — | — |
| GATE 6 | A-15, A-16, A-17, A-18 | todo | — | — | — |

## Quality items

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| Q1 | (not described in the master prompt — to confirm) | blocked | — | — | owner defines |
| Q3 | Acceptance rule per (model × technique): McNemar p ≥ 0.1, drop ≤ 1 pt, saving CI > 0 | todo | sdc-v3/W1 | — | implemented in the bench gate |

## G-items without a description in the master prompt

G28, G29, G30, G31, G32, G33, G38, G40 — `blocked` until the owner supplies their definition (the full
Bengali plan `docs/plan/SDC-PLAN-v3.md` is not in the repository).

## Phase 7 — Final proof

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| U0b | Re-run U0 | todo | — | — | — |
| FINAL | Final bench + `SDC-v3-final-report-bn.md` | todo | — | — | — |

## Deferred (D-1: local models out of scope for now)

| ID | Item | Status | Note |
|---|---|---|---|
| W2 | Ollama `num_ctx`, qwen3.5:9b | deferred | keep the local backend trait clean |
| I6-local | Local summariser of I6 | deferred | cheap API model chosen by the router instead |
| L6 | Small custom model | deferred | — |
