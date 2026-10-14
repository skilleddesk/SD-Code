# SDC-v3 — Progress

Source of truth for resuming work on SDC-PLAN-v3. A new session reads this file first.
Status values: `todo` · `in-progress` · `blocked` · `done` · `killed` (with data) · `deferred`.
Branch naming: `sdc-v3/<ID>`. Numbers come only from `bench/results/` (never typed by hand).

- Current phase: **0 — Preparation** → waiting at **GATE 0** (`docs/reports/gate-0-bn.md`)
- Baseline: tag `sdc-baseline-v1` = `274ec6a` (= `v1.0.0`), binary `bench/bin/v1/sdcd` (not committed)
- Assumption register: `docs/plan/ASSUMPTIONS.md`
- Next action: owner answers the open decisions in `gate-0-bn.md` and says "continue" → Phase 1 (W1)

## Phase 0 — Preparation

| ID | Item | Status | Branch | Numbers / evidence | Next action |
|---|---|---|---|---|---|
| P0.1 | Tag HEAD `sdc-baseline-v1` | done | — | tag on `274ec6a` (local, not pushed) | push the tag after owner OK |
| P0.2 | Build v1 into `bench/bin/v1/` | done | — | macOS arm64 release build from the tag; `cargo test` 738 pass / 0 fail / 6 ignored | Windows/Linux v1 builds in CI (Phase 1) |
| P0.3 | Verify A-01 (code facts) | done | sdc-v3/P0 | see ASSUMPTIONS A-01 | — |
| P0.4 | Existing-capability check | done | sdc-v3/P0 | gate-0-bn.md §2 | — |
| P0.5 | U0 uniqueness audit | done | sdc-v3/P0 | `docs/reports/U0-uniqueness-bn.md` | re-run in Phase 7 |
| P0.6 | Open decisions to owner | blocked | — | gate-0-bn.md §5 | owner answers |
| GATE 0 | Gate report | in-progress | sdc-v3/P0 | `docs/reports/gate-0-bn.md` | wait for "continue" |

## Phase 1 — Measure

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W1 | `sdcd bench` (fixtures, 40 tasks, runner, ledger, search bench, competitors, tiers, `--compare`, `--report-bn`) | todo | sdc-v3/W1 | — | after GATE 0 |
| GATE 1 | A-02, A-03, A-09, A-10, A-14 measured | todo | — | — | — |

## Phase 2 — Durable foundation

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| R1 | Journal (`job_events` intent/result, SQLite WAL) | todo | sdc-v3/R1 | — | — |
| R2 | Resume on start | todo | sdc-v3/R2 | — | — |
| R3 | Offline queue (backoff + jitter) | todo | sdc-v3/R3 | — | — |
| R5 | Graceful shutdown on sleep/shutdown | todo | sdc-v3/R5 | — | — |
| W7a | Checkpoints (hidden git refs / CAS blobs, undo turn, timeline, VPS) | todo | sdc-v3/W7 | — | — |
| W9 | Security basics (OS key storage + migration, rate limiter, untrusted tagging, secret redaction, SQLite integrity + backup) | todo | sdc-v3/W9 | — | — |
| GATE 2 | A-19 local part | todo | — | — | — |

## Phase 3 — Speed and intelligence

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W3 | Search module `sdcd/src/search/` | todo | sdc-v3/W3 | — | — |
| W4 | Code intelligence (tree-sitter, symbols, repo map) | todo | sdc-v3/W4 | — | — |
| W5 | Cache layout + tool pruning | todo | sdc-v3/W5 | — | — |
| I1 | Context Ledger | todo | sdc-v3/I1 | — | — |
| I2 | Skeleton-first reads | todo | sdc-v3/I2 | — | — |
| I10 | Token Receipt | todo | sdc-v3/I10 | — | — |
| Q2 | Model profiles (generated) | todo | sdc-v3/Q2 | — | — |
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
| I5 | Budget Contract | todo | sdc-v3/I5 | — | — |
| I9 | State Capsule failover | todo | sdc-v3/I9 | — | — |
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
| I6 | Privacy Split Router (cloud-only part) | todo | sdc-v3/I6 | — | — |
| I7 | Fleet Fan-out | todo | sdc-v3/I7 | — | — |
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
