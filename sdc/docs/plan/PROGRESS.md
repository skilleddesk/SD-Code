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
| W7a | Checkpoints (hidden git refs / CAS blobs, undo turn, timeline, VPS) | done (CAS layer added) | sdc-v3/W7 | v1's shadow-git checkpoints, rewind stack, checkpoint.list timeline and VPS snapshots stay; new: every write and edit keeps the file's old content in `.sdc/objects/<sha>` with `.sdc/objects/turns/<turn>.json`, so a turn is taken back in a project with no git (switch `v3.cas`, on) - test: write through the tool, undo_turn restores it, a file the turn created is removed | hidden git refs per checkpoint; the store's size in Settings |
| W9 | Security basics (OS key storage + migration, rate limiter, untrusted tagging, secret redaction, SQLite integrity + backup) | partial | sdc-v3/W9 | secret redaction (token shapes + named high-entropy assignments → ‹secret:N›, restored on run/write, vault kept in the journal for resume) - test: a fake key never reaches the provider; untrusted marking of web/browser/MCP text; per-host token bucket (120/min, burst 20); quick_check before the daily copy (daily backup already existed); switches `v3.redact`/`v3.untrusted`/`v3.rate_limit` (on) | Linux Secret Service key storage (keys are 0600 files there today); Windows DPAPI/macOS Keychain already used by the packaged app |
| GATE 2 | A-19 local part | todo | — | — | — |

## Phase 3 — Speed and intelligence

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W3 | Search module `sdcd/src/search/` | done | sdc-v3/W3 | 52,501 files, p50: filename 2.5-3.2 ms (v1 229-234), text 2.0-6.6 ms (v1 227-1204 incl. a no-match full scan); first call 150 ms (v1 229-2815); peak RSS 58 MB (`results/2026-10-14-search-v1` vs `-v3`); `ignore` parallel walk + .gitignore, notify-patched index, byte prefilter, trigram Bloom content index; switch `v3.search` (on) | Windows/Linux numbers in CI; `nucleo` fuzzy names + UI debounce; same engine via `sdcd search` on a VPS |
| W4 | Code intelligence (tree-sitter, symbols, repo map) | built (pattern extractor) | sdc-v3/W4 | `intel::symbols` names declarations in 10 language families (name, kind, line); `find_symbol` and `symbols` tools (switch `v3.intel`, off: they add tools); `repo_map` ranks changed/named/most-declared files within a token budget (switch `v3.repomap`); end-to-end test: switch off = the request reads as before | tree-sitter grammars behind the same `symbols()` seam (the extractor is line-pattern, not a parser); a repo map that updates as files change |
| W5 | Cache layout + tool pruning | built, off (awaits Q3) | sdc-v3/W5 | `stable_prefix`: the date leaves the system prompt for the turn's message; `tool_pruning`: intent tool sets (core + web/browser/process/remember by the request's words), a called tool joins the list; test proves both change only what they say | paid bench comparison per model (Q3); Gemini context caching; Anthropic 4-breakpoint layout |
| I1 | Context Ledger | built, off (awaits Q3) | sdc-v3/W5 | `ledger_diff`: a changed whole-file re-read goes back as a unified diff (≤60% of the file), cleared on fold; v1's unchanged-read stub kept | `recall(Lk)` tool; refs on stubs |
| I2 | Skeleton-first reads | built, off (awaits Q3) | sdc-v3/W5 | `skeleton`: files >200 lines read whole answer an outline (imports summed, declarations with line numbers; 12 language families) when it saves ≥40% | tree-sitter symbols (W4); expand by symbol |
| I10 | Token Receipt | done (footer) | sdc-v3/W5 | every agent turn's footer: input by part (apportioned to the provider's count) and tokens saved per technique (unchanged re-reads, diffs, tool pruning); switch `v3.receipt` (on) | monthly "saved $X"; dev-only shadow baseline; a UI card |
| Q2 | Model profiles (generated) | done (mechanism) | sdc-v3/W5 | `protocol/model-profiles.json` written only by `sdcd bench profiles` from Q3 verdicts; per-turn switches settle setting → profile → default; unknown models: no token technique | first real profile needs the paid comparisons; best edit format per model |
| L0 | Memory | built, off (awaits Q3) | sdc-v3/W4 | `v3::memory::select` sends only the facts the request touches (plus the newest few), `consolidate` keeps one wording of a repeated fact; wired where the kernel reads `.sdc/memory.md`; switch `v3.memory` (a token technique, off) | memory that SDC rewrites itself (today only the person's `remember` writes it) |
| L1 | Lessons | built, off (awaits Q3) | sdc-v3/W4 | a step that failed and the step that worked after it become a lesson in `.sdc/lessons.md`; a matching request is sent the best one as data; a lesson that warns about a step which fails again while it is sent is taken back; switch `v3.lessons` | A-?? needs a paid run; lessons from what the person said ("no, not that") |
| GATE 3 | A-04–A-08, A-11; input −35% vs v1; Q3 on 5 models | todo | — | — | — |

## Phase 4 — Quality and cost

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W6 | Router (main + fast model, estimateCost wiring) | done | sdc-v3/W6 | setting `router.fast` (`provider:model`): a question with no file named and no work asked goes to the fast model, everything else stays on the main one (switch `v3.router`, on; nothing happens until the setting is filled); every turn's footer carries an estimate from the model's price and the turn's size | the fast model's cost beside the main one in the receipt; routing on the turn's measured size |
| W7b | Verify loop + `/plan` | done | sdc-v3/W7 | auto_check drives a turn to green in up to two rounds (v1); new: `/plan` (or `@plan`) answers with the plan and refuses every writing tool for that turn - end-to-end test: the edit is refused, the file is untouched, the plan comes back | a UI that takes the plan's go-ahead; a verify loop that runs the project's own test command by name |
| I3 | Draft–Verify cascade | done | sdc-v3/W7 | setting `cascade.strong` (`provider:model`): when the draft model is stuck (the same step failed twice, or three failures in one turn) the strong model takes the task over from a capsule of the state, not from the draft's conversation (switch `v3.cascade`, on); end-to-end test: two failing draft steps, then the strong model finishes and the person sees its answer | a cascade that triggers on a failed check rather than on stuck steps |
| I4 | Recipe Memory (= L2) | built, off (awaits Q3/A-13) | sdc-v3/I4 | after a turn whose change was followed by a passing command, a ≤300-token recipe (task, changed files, proving commands, route) in `.sdc/recipes/*.md`; a request sharing ≥3 keywords (and ≥¼ of its own) gets it as data; switch `v3.recipes` | A-13 needs the paid `--tier repeat` run; embeddings ranking |
| I5 | Budget Contract | done | sdc-v3/I5 | `@budget 40k` / `@budget $0.10`: opening note, converge at 80%, stop at 100% with the work kept (end-to-end test); switch `v3.budget` (on) | a live gauge in the UI |
| I9 | State Capsule failover | done | sdc-v3/I5 | `v3.failoverOrder` (`provider:model, …`): a provider that fails for good hands the task to the next, as a ≤6,000-char capsule (goal, open plan steps, changed files, last 12 steps, last words); fault-injection test (401 → next model finishes) | periodic capsules every few steps (today built at the failure); cross-provider journal resume |
| Q4 | Runtime quality guard (SPRT) | done | sdc-v3/W7 | bench::sprt decides as the pairs arrive (better / not better / take more), printed per model by `sdcd bench profiles` and recorded in every Comparison; `sdcd bench drift --labels A,B,C` alarms when a model's pass rate slips away from the rate it was chosen for (CUSUM). A five-point effect needs about 120 winning pairs - the test says so | the alarm moving a model's profile on its own |
| Q5 | Lift weak models | done | sdc-v3/W7 | a model with a small name or a cheap price is given room to think (`reasoning_effort: high`, or the provider's thinking mode) and one instruction to lean on the tools - read the real lines, run the check, say what is still unsure (switch `v3.effort`, on); strong models are left exactly as they were | a paid comparison of lifted against unlifted; per-model effort in the profiles file |
| L2 | Recipes (see I4) | built, off | sdc-v3/I4 | see I4 | see I4 |
| L3 | External knowledge | built, off (awaits Q3) | sdc-v3/W4 | every `web_fetch` keeps the page (address, title, date, words, 1,500-char excerpt) in `.sdc/knowledge/`; a later request sharing 3+ words gets the excerpt with its address and age, and a warning past 30 days; switch `v3.knowledge` | a paid run; embeddings for better matching |
| GATE 4 | A-12, A-13; Q3 on all models | todo | — | — | — |

## Phase 5 — Cross-device and security

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| X1 | Login (passkeys, magic link, TOTP) | done | sdc-v3/X3 | the sign-in route asks for a six-digit code once a secret is set, and every owner action but enrolling needs one (`cloud/src/owner.ts`); a code's 30-second step works once (`totp_steps`, `0003_owner_mfa.sql`) and the secret is only stored after a code made from it comes back (`cloud/test/owner.test.ts`) |
| X2 | Device enrolment + recovery key | done | sdc-v3/X3 | enrolment: `sdcd/src/anywhere/registry.rs` (pairing nonce, add/claim/revoke device); the pairing QR is the pinned key (`web/src/crypto/session.ts`); recovery: `web/src/transport/recovery.ts` with `web/e2e/recovery.e2e.test.ts` and `web/e2e/ui.recovery.e2e.test.ts` (a lost device is recovered and the person is told) | rotating the relay secret when an account is deleted (see G36) |
| X3 | E2E sync | done | sdc-v3/X3 | sealed vault blobs (cloud), the daemon's own crypto (`sdcd/src/anywhere/crypto.rs`, 817 lines), the remote workspace over the relay, and `web/e2e/workspace.e2e.test.ts` + `web/test/vault.test.ts` | revision-based conflict rules; a sync-status screen |
| X4 | Handoff (leases + fencing) | done | sdc-v3/X3 | the relay is what hands out the fence (`hub.ts`: a `leases` table and `handoff.claim`/`release`/`state`); the daemon stamps every frame with it and drops a frame from behind it (`anywhere/core.rs`), and the web client stamps the fence it was given (`web/src/transport/link.ts`). Tests: `cloud/test/handoff.test.ts`, `anywhere::core_tests`, `web/test/handoff.test.ts` |
| X5 | Hardening (pinning, revoke/wipe, signing, notarization, dep scan) | done, bar the Apple secrets | sdc-v3/X5 | wipe: the account asks the computer, which writes the erase marker and erases everything at its next start (`reset::mark_for_erase`, `askWipe`); dep scan: `pnpm audit` + `cargo audit` in CI; signing and notarization are wired in `release.yml` and turn on the moment the Apple Developer ID secrets are set - until then the macOS build is ad-hoc signed, which the release notes say |
| X6 | Phone browser | done | sdc-v3/X3 | `web/src/ui/` (chat, files, code, approval cards) in a phone-sized layout, Bangla and English strings (`web/src/i18n.ts`), push (`web/src/push/`), and `web/e2e/ui.e2e.test.ts` (61 tests in 7 e2e files, all passing) | a real phone over a real network is still to be tried |
| R4 | VPS Runner | done | sdc-v3/R4 | the machine is asked over ssh before a turn starts (`v3::runner::reachable`); if it is asleep, off the network or no longer takes the key, the turn stays on this machine and says why. A remote turn is journaled with the machine it ran on, and a turn lost there is reported rather than started again here (`v3::journal::lost_remote`) |
| G36 | Data export / delete / retention | done | sdc-v3/X3 | `/api/me/export`, `/api/me/delete` (which also asks the computer to wipe itself) and the hourly retention purge, all behind a sign-in link plus, when set, the second factor |
| GATE 5 | A-20, A-21, ASVS L2 | todo | — | — | — |

## Phase 6 — New capabilities

| ID | Item | Status | Branch | Numbers | Next action |
|---|---|---|---|---|---|
| W8 | `/research` | v1 + terms | sdc-v3/W4 | the research turn (sub-agents, page fetchers, its own report) is v1's; new: every page it reads goes through the same robots.txt door (G37) | the research turn's own token receipt; a research cache that survives the turn (L3 covers pages, not whole runs) |
| I6 | Privacy Split Router (cloud-only part) | done (cloud part) | sdc-v3/I6 | `secret_paths` in `.sdc/policy.toml`: reads refused, grep/search lines dropped, commands naming a secret file have output withheld; `.sdc/audit/sent.jsonl` records files sent per provider/model; canary end-to-end test (read + grep + cat) → 0 leaks; switch `v3.privacy` (on) | 100-task canary run on the bench; embeddings (none exist yet); local summariser deferred (D-1) |
| I7 | Fleet Fan-out | built (switch off by default) | sdc-v3/I7 | `fleet_run` tool: parallel SSH, outputs normalised (times, PIDs, ids, IPs, durations, host names) and clustered, majority sample + diffs; asks before non-read-only commands; canary-first; synthetic A-16: 20 servers −80.4%, 0 hidden diffs (`results/2026-10-14-fleet-synthetic`); switch `v3.fleet` (off: it adds a tool) | real Docker 5/10/20 fleet; `@fleet` chat syntax |
| I8 | Interlingua | built, off (awaits Q3) | sdc-v3/W4 | one English brief per non-English message: a built-in Banglish/Hinglish dictionary plus the person's own glosses (`kaj (work)`) kept in `.sdc/interlingua.json`; paths, identifiers and options are never translated; an English message is untouched; switch `v3.interlingua` | a paid run; more languages than Banglish/Hinglish |
| I11 | Self-Improving Knowledge Core (umbrella over L0–L5) | done | sdc-v3/W4 | one door for every learned fact: two sightings from separate turns, no instruction-shaped text, no contradiction with what is admitted, right size; admitted items are kept with their evidence, refusals with their reason; `rollback` takes back what proves wrong; nightly pass over the day's candidates; A-18 test: 50 poisoned proposals → 50 blocked, the one honest fact stands; switch `v3.knowledge_core` (on) | wiring the model profiles (Q2) through the same door; a UI for what SDC learned |
| I12 | (definition to confirm with owner — not described in the master prompt) | blocked | — | — | owner defines |
| PAR | Parallel agents in worktrees + Agents board | done (Workbench ▸ Agents) | sdc-v3/parallel | `v3::parallel`: an agent gets its own checkout of the project's own repository (`git worktree add --detach`, kept under the daemon's per-project directory, never inside the person's folder), an id from the request's first words, and a card in `.sdc/agents/` (request, worktree, started, state, note); `board` lists them newest first, `finish` ends one, and taking a card off the board never deletes the work; protocol `agents.board`; tests: two agents never share a name or a directory, and the worktree is a real checkout | nothing: a card is never deleted with its worktree, and `parallel.max` (3) is the only cap |
| AUTO | Automations + Inbox | done (Workbench ▸ Inbox) | sdc-v3/automations | `v3::automations`: automations in `.sdc/automations.toml` (a name, a request, `every = 6h` or `at = 09:30` local), `due` decides what has come round, the answer is written into `.sdc/inbox/<instant>-<name>.md` with its own front matter, and the last run of each is remembered in `.sdc/inbox/.state.json`; a schedule SDC cannot read is refused with a message instead of never running; protocol `inbox.list`; a disabled automation is still the person's | nothing: the clock is `v3::run::spawn` in the daemon, through the same `automation.run` the Inbox presses |
| L4 | Contextual-bandit router | built, off (awaits Q3) | sdc-v3/W4 | `v3::bandit`: per kind of request (question / read / search / change) each model's record is kept (did the turn finish, and what did it cost), an untried arm is tried first and UCB1 picks after that; recorded in `.sdc/bandit.json`, wired where the router picks the model (switch `v3.bandit`, off - a token/cost technique until Q3 passes) | a UI that shows the ranking; the bandit choosing the thinking effort too |
| L5 | Prompt / tool-description optimisation | done | sdc-v3/L5 | `agent::l5_budget`: the whole tool block, the largest single tool and the system prompt are measured by tests, so a paragraph that grows is caught at once; the `browser` description was the largest by far and now says the same in fewer words |
| G34 | UI end-to-end tests on 3 OS | done | sdc-v3/G34 | `.github/workflows/window-check.yml` drives the real window on all three systems; since 2.0 the app bundle smoke and the a11y run are also a CI step on every push (`ci.yml`, G34) | nothing |
| G35 | Opt-in privacy-safe telemetry / crash reports | done (crash reports on request; no telemetry, by design) | sdc-v3/G35 | crash reports are written on this machine and leave only when the person presses send; the About tab’s decorative telemetry switch is gone and the row states that nothing is sent on its own | an ingest endpoint if the project ever wants usage data |
| G37 | Respect site terms (research) | done | sdc-v3/W4 | `sdcd/src/v3/terms.rs`: the site's `robots.txt` is read once per host before a page, SDC's own section and the wildcard's both apply, a disallowed path is refused quoting the rule, a `Crawl-delay` is waited out (switch `v3.terms`, on) | paywall/login walls (a page a person is entitled to but SDC cannot reach); a per-host politeness budget for research runs |
| G39 | Accessibility + first-run onboarding | done | sdc-v3/G39 | `app/src/kernel/Onboarding.tsx` - the first-run wizard (folder, server or new project, then the first task); `app/scripts/a11y-bundle.mjs` runs axe over the built bundle | nothing |
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
| U0b | Re-run U0 | done | sdc-v3/X3 | `docs/reports/U0b-uniqueness-bn.md`: local execution and resume are now common (Claude Code `--resume`, Cline checkpoints, Aider `--restore-chat-history`), so the honest claim is the model key staying in your own daemon with an E2E phone client; source is vendor docs pages, search engines were unavailable, confidence noted per row |
| FINAL | Final bench + `SDC-v3-final-report-bn.md` | todo | — | — | — |

## Deferred (D-1: local models out of scope for now)

| ID | Item | Status | Note |
|---|---|---|---|
| W2 | Ollama `num_ctx`, qwen3.5:9b | deferred | keep the local backend trait clean |
| I6-local | Local summariser of I6 | deferred | cheap API model chosen by the router instead |
| L6 | Small custom model | deferred | — |
