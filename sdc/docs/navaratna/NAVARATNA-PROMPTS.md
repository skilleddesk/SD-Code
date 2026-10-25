NAVARATNA PROMPTS — English prompts and task cards for SDC
Oct 24, 2026 · @Mehedi Hasan
How to use
Save the plan doc as NAVARATNA.md and this doc as NAVARATNA-PROMPTS.md in SDC's skills folder, next to MEGHNA. Then type one of these lines to SDC (or Claude Code). The word NAVARATNA makes the agent load both files before anything else.
You type
What the agent does
NAVARATNA next
Picks the first todo task in the plan's recommended order whose dependencies are all done, runs its card below.
NAVARATNA N4-2
Runs only that card. Stops and reports in Bengali if a dependency is not done.
NAVARATNA status
No code changes. Reports progress in Bengali from the tracker and git log.
NAVARATNA audit
No code changes. Checks which of problems P1–P19 are still open in the code.
NAVARATNA <free text>
Maps the sentence to one task ID in one line, then runs it; asks one question if nothing matches.
Tip: one task per turn. If a card is large, the agent finishes a sub-step, marks in-progress (step n/m), and stops. Type NAVARATNA next again to continue.
Skill frontmatter and master prompt
Put this frontmatter at the top of NAVARATNA.md:
---
name: navaratna
description: "NAVARATNA plan for SDC: nine pillars that let a local 9B model finish large projects and long-context work (window truth, spec vault, clean context, surgical edits, proof-first, exactly-once, many tries, smart relay, learn and measure). Builds on MEGHNA (Silent Relay M1, Brick Engine M7). Whenever the user's message contains the word NAVARATNA, read this file and NAVARATNA-PROMPTS.md (same folder) completely before doing anything else."
---
Master prompt (the agent follows this on every NAVARATNA turn):
You are working on SDC under the NAVARATNA plan.

1. Read NAVARATNA.md and NAVARATNA-PROMPTS.md in full. Also read MEGHNA.md section 0 (rules R1–R11) if a card depends on an M-task.
2. Decide the task:
   - "NAVARATNA next": walk the plan's recommended order; take the first task whose status is todo and whose dependencies are all done (M-dependencies are checked in MEGHNA's tracker).
   - "NAVARATNA <ID>": take only that task; if any dependency is not done, stop and report which one, in Bengali.
   - "NAVARATNA status" or "audit": do not change code; report in Bengali.
   State the task ID and name in one line before starting.
3. Run `git status`. If the tree is dirty, stop and report.
4. Open the task card in NAVARATNA-PROMPTS.md. Read every file listed under READ before writing to any file.
5. Work in small steps. After each step run the related tests. Use the scripted fake engine and VCR fixtures for model behaviour; never claim live 9B results.
6. Run every command under VERIFY and record the real output. Anything not run is INCOMPLETE; anything the owner must run is OWNER-TO-RUN.
7. Put every new behaviour behind its nr.* switch with the default given in the card. Write honest switch descriptions.
8. Update the tracker row (status, commit) in NAVARATNA.md and add one line to CHANGELOG.md under Unreleased.
9. Commit once: navaratna(<ID>): <short English summary>.
10. Report in Bengali using the plan's report format. Then stop. One task per turn.
Shared rules every card inherits
These apply to every card below; cards only list what is specific to them.
• Honesty (R1): never call something done that you did not run. Use INCOMPLETE and OWNER-TO-RUN.
• Secrets (R2, N7): never print or store keys, tokens or passwords: not in chat, logs, commits, fixtures, capsules or Golden Traces.
• Manual Pin (R3): if the user picked a cloud, API or CLI model, behaviour is unchanged. Zero calls outside the chosen model.
• Switches (R4, N1): each new behaviour sits behind its nr.* switch, default as the card says. Only pure safety or record-keeping switches default on.
• Protocol (R5): any new method or event goes into protocol/sdcp.schema.json, protocol/types.ts and daemon dispatch; then node protocol/check.mjs must pass.
• Privacy (R7, N6): privacy = "local-only" or files in secret_paths never leave the machine. Cloud Blueprint or relay only with relay.auto consent and inside cost limits.
• No live 9B yet (R8): use the scripted fake local engine and VCR cloud fixtures. Mark live parts OWNER-LATER.
• Edits (N3): model-written code is never embedded in JSON strings. The model returns a plain fenced code block; SDC code places it.
• Tests (N4): model-written tests may never modify or delete anchor tests (pre-existing, human-written, or previously passing).
• Parallel (N5): no concurrent 9B calls unless the memory budgeter (N1-3) approves; each concurrent step uses its own git worktree.
• Numbers (N2): no improvement number in code comments, docs or UI unless produced by the bench.
• Slow drive (R11): the repo is on an external SD drive. Scope find/grep to folders. A timeout is INCOMPLETE, not a pass.
• Language (R10): reports to the owner in Bengali; code, commits, file names and commands in English; every new UI string in strings.ts with a Bengali entry in locales/bn.
Card format: Goal · Read · Do · Don't · Verify · Switch · Done when.
Task cards: pillars 1–3
N1-1 — num_ctx lock + window probe (K1)
• Goal: no local request can be silently truncated; SDC knows the real usable window.
• Read: the Ollama provider module (locate with a scoped grep for num_ctx / /api/chat), guard/context.rs, sme/budget.rs, MEGHNA M7-1 Window Meter code.
• Do: (1) send options.num_ctx on every native Ollama request, never via the OpenAI-compatible endpoint; (2) before each request, count prompt tokens with the shared estimator; if prompt + reserved output exceeds num_ctx, refuse and hand back NeedsSplit instead of sending; (3) add sdcd probe window --model <m>: plant key-value facts at start, middle and end of growing prompts and record the largest length with full recall; store per model in ~/.sdc/models/<m>.toml; (4) when no probe exists, use 50% of the advertised context.
• Don't: change cloud providers; guess VRAM.
• Verify: unit tests for the pre-flight check (over/under limit) with the fake engine; cargo test --locked; probe runs against the fake engine and writes the file.
• Switch: nr.ctx_lock (on).
• Done when: zero code paths call Ollama without num_ctx; an oversize prompt produces NeedsSplit, never a send.
N1-2 — Quant and KV advisor (K2)
• Goal: warn when the local model setup harms long work.
• Read: N1-1 output, Settings → Models UI code.
• Do: read ollama show details (quantization, context, KV cache type if exposed); show a one-line advice in Settings and in the Brick big-prompt card: prefer Q5_K_M or higher when memory allows; never 4-bit KV cache for long tasks; if Q4 is in use, lower the step target from 80% to 60% of the window.
• Don't: change the user's model or download anything.
• Verify: UI string tests (en + bn); unit test of the step-target rule.
• Switch: nr.quant_advice (on, advice only).
• Done when: advice appears for a Q4 fixture and not for a Q8 fixture.
N1-3 — Memory budgeter (K3)
• Goal: decide safely how many concurrent 9B slots and how much context fit.
• Read: N1-1, N1-2, hardware detection code.
• Do: compute kv_budget = free_memory − weights − overhead; tokens_total = kv_budget / bytes_per_token; allow slots = 2 only if each slot still gets ≥ the probed usable window; otherwise slots = 1. Expose the result to the scheduler.
• Don't: set OLLAMA_NUM_PARALLEL for the user; just read and advise.
• Verify: table-driven tests for 8 GB, 16 GB, 24 GB fixtures.
• Switch: nr.mem_budget (off).
• Done when: the scheduler never starts a second 9B call when the budgeter says 1.
N2-1 — Parser, sentence coverage, R-IDs (K4, K6)
• Goal: every sentence of a big prompt is accounted for before planning.
• Read: MEGHNA M7-2 Spec Vault code, Requirement Ledger schema.
• Do: (1) deterministic block parser: headings, lists, tables, fenced code, JSON and quoted blocks stay whole as one unit; conditional sentences (if/else, "unless", "যদি…তবে") stay whole; (2) number units S-001…; (3) the 9B (or Blueprint, later N2-3) labels each unit requirement or background; units become R-IDs with source spans; (4) coverage gate: 100% of units must be mapped or planning is blocked with a list of unmapped units; (5) store the map in the vault.
• Don't: ever drop or summarise a unit; split inside code blocks.
• Verify: fixtures with code blocks, tables, Bengali and Banglish text, nested conditions; assert 100% coverage and zero split code blocks.
• Switch: nr.sentence_cover (on, verification only).
• Done when: a fixture with one deliberately unmapped unit blocks planning and names it.
N2-2 — Language normalisation hardening (K7)
• Goal: Bengali or Banglish prompts become a precise English spec without losing meaning.
• Read: MEGHNA's 4-line spec code, N2-1.
• Do: per unit (not whole prompt), produce an English line; keep the original in the vault; back-check that every number, name, path and code identifier in the original appears in the English line; flag mismatches.
• Don't: translate code, identifiers or quoted strings.
• Verify: fixtures with mixed Bengali/English and numbers.
• Switch: nr.lang_spec (off).
• Done when: the identifier/number back-check passes on all fixtures.
N2-3 — Hybrid Spec Compiler + cloud Blueprint (K5)
• Goal: for large or tangled prompts, one cloud call produces the architecture skeleton; 9B does the rest.
• Read: N2-1, MEGHNA M1-3 Capsule v2, M1-4 cloud executor, rotation config.
• Do: trigger only when the prompt exceeds a size or complexity threshold AND relay.auto consent exists AND privacy allows; send the parsed units (secrets redacted) to the chain row tagged architect; require output as a Blueprint file: folder tree, data models, interface contracts (empty signatures), R-ID list with unit mapping; validate with the N2-1 coverage gate and contract syntax check; on failure fall back to the local path.
• Don't: let the Blueprint write implementation bodies; skip validation.
• Verify: VCR fixture for the cloud call; tests for consent-off, local-only and validation-failure paths.
• Switch: nr.blueprint (off).
• Done when: with consent off or local-only, zero cloud calls happen.
N3-1 — Fresh step + edge pinning (K8, K9)
• Goal: every step runs in a new, short prompt built from the ledger, never a growing chat.
• Read: MEGHNA M7-4 Context Pack Builder, runtime/step.rs.
• Do: build each step prompt from: rules prefix, goal + R-IDs + done-when (top), step units, contracts, last Handover Notes, then repeat goal and done-when in one final line; no previous chat turns.
• Don't: drop spec units to fit; return NeedsSplit instead.
• Verify: snapshot tests of built prompts; assert no prior-turn text appears.
• Switch: nr.fresh_step (off).
• Done when: step prompt size is independent of how many steps came before.
N3-2 — Skeleton Context Pack (K10)
• Goal: the model sees signatures, not whole files.
• Read: N3-1, N4-1.
• Do: for each related file include only its skeleton (imports, type and function signatures with line numbers) from N4-1; include the full body only for the target symbol(s); drop look-alike but irrelevant symbols ranked low by ranked_search.
• Don't: include whole files above a small size threshold.
• Verify: pack-size tests on a large fixture repo; target body always present.
• Switch: nr.skeleton_pack (off).
• Done when: pack token count drops on the fixture while the target body is present in 100% of cases.
N3-3 — Error Slicer (K11)
• Goal: the model sees the real error, not 200 lines of noise.
• Read: verify/, compiler/test output handling, MEGHNA compressor.
• Do: parsers for rustc/cargo, tsc, eslint, pytest, jest, go: extract first error message, file:line, and ±10 lines of code; for test failures include assertion, expected vs actual; keep the full log in the vault (spec_read can fetch it).
• Don't: hide the existence of other errors; append a count ("+7 more errors").
• Verify: golden-file tests with real log samples per tool.
• Switch: nr.error_slice (off).
• Done when: every sample produces a slice under a fixed token cap that contains the correct file:line.
Task cards: pillars 4–5
N4-1 — Tree-sitter symbol index and skeleton generator (K10, K12)
• Goal: SDC can list, locate and slice every symbol in supported languages.
• Read: existing Code Graph module, ranked_search, Cargo.toml dependencies.
• Do: add tree-sitter grammars for the languages SDC already supports (start with Rust, TypeScript/JavaScript, Python); build an index of symbols (kind, name, file, byte range, signature); generate skeletons (imports + signatures + line numbers); reuse or merge with Code Graph instead of duplicating.
• Don't: block on unsupported languages; mark them no-grammar and fall back.
• Verify: index tests on fixture files per language; skeleton snapshot tests; R9 Cargo.lock rule.
• Switch: nr.symbol_index (on, index only; no prompt change).
• Done when: for each fixture, every top-level function and method is found with the correct byte range.
N4-2 — Symbol Patcher + edit-mode chooser (K12)
• Goal: the 9B never rewrites whole files.
• Read: N4-1, current edit_file / apply_patch tools, Step Transaction.
• Do: new edit tool replace_symbol: the model names a symbol and returns the new definition as a plain fenced code block (not JSON); SDC swaps the byte range and re-indents. Keep search/replace for small in-body edits and inserts. Chooser rule: whole-symbol rewrite when the change touches most of a symbol; search/replace for a few lines; whole-file only for new or very small files. Reject any output containing elision markers (// ..., # rest unchanged, "existing code").
• Don't: accept a patch that changes symbols other than the named one.
• Verify: tests for replace, insert-after, rename-rejection, elision-rejection, indentation; fake-engine end-to-end edit.
• Switch: nr.symbol_patch (off).
• Done when: elision outputs are always rejected and other symbols in the file are byte-identical after a patch.
N4-3 — Parse/lint edit gate (K13)
• Goal: a syntactically broken edit never lands on disk.
• Read: N4-1, verify module, Step Transaction.
• Do: after composing an edit in memory, parse the new file with tree-sitter (error nodes = fail) and run the fast linter if configured; on failure discard the edit and return a short error (via N3-3 when available) to the model.
• Don't: run the full build here; that stays in verify.
• Verify: tests with broken braces, bad indentation (Python), unterminated strings.
• Switch: nr.edit_gate (on).
• Done when: none of the broken fixtures reach disk.
N4-4 — Output chunking + continuation (K14)
• Goal: long outputs never stop silently.
• Read: MEGHNA M7-3 Chunk Planner.
• Do: per-step output cap; long documents written per section with <<SECTION n END>> markers; missing marker = incomplete → automatic continuation call with the last 30 lines as context; sections stored as separate files and joined by a manifest.
• Don't: join sections whose markers are missing.
• Verify: fake engine that stops mid-section; assert continuation and correct join.
• Switch: nr.out_chunk (off).
• Done when: a truncated fixture is always completed or reported, never joined half-done.
N5-1 — Test-first with anchors and stub check (K15)
• Goal: catch logic bugs that compile, without trusting bad AI tests.
• Read: N4-2, verify module, project test layout.
• Do: for steps tagged code with a testable done-when: (1) fresh 9B call writes 1–5 small tests from the R-ID's done-when only (not from the code); (2) SDC runs them against a stub (unimplemented) version: each new test must FAIL, otherwise discard it as vacuous; (3) separate fresh call writes the code; (4) run new tests plus all anchor tests; anchor tests must stay green and untouched; (5) a new test that keeps failing while all anchors pass and the code looks correct is flagged for review, not silently deleted.
• Don't: let the code-writing call see or edit the new tests; use test-first for trivial steps (config, renames).
• Verify: fixtures: vacuous test rejected, anchor protected, wrong test flagged.
• Switch: nr.test_first (off).
• Done when: no run can turn green by editing or deleting an anchor test.
N5-2 — Existence gate for packages and APIs (K16)
• Goal: invented packages or methods are caught before build.
• Read: MEGHNA M4-1 (verify.api_exists), lockfiles, N4-1 index.
• Do: for each new import, check the lockfile/manifest; for each called member on a project symbol, check the index; for external packages, check an offline list of installed exports when available. Unknown names → warning attached to the step; with nr.exist_gate=block (future) → fail.
• Don't: add network lookups to registries by default.
• Verify: fixtures with fake package and fake method.
• Switch: nr.exist_gate (on, warn only).
• Done when: both fakes are flagged and real ones are not.
Task cards: pillars 6–7
N6-1 — Artifact Registry + duplicate detection (K18, K19)
• Goal: nothing is built twice and every R-ID has exactly one owner.
• Read: MEGHNA M7-2/M7-6 ledger and coverage code, N4-1 index.
• Do: (1) one-owner check at planning: each R-ID assigned to exactly one step, else plan rejected; (2) registry table: artifact (file/symbol/section), R-ID, step ID, content hash; (3) before a create step, look up registry and index: if the name exists, convert the step to an edit and record why; (4) after each step, scan for same-name symbols and near-identical bodies (normalised hash); report as a warning with both locations.
• Don't: auto-delete duplicates.
• Verify: fixtures: double ownership rejected, create→edit conversion, duplicate body detected.
• Switch: nr.registry (on, record and warn only).
• Done when: the final receipt lists every R-ID with step, model and evidence (file:line or test name).
N6-2 — Journal + Git Shadow (K20)
• Goal: any failed step returns the tree to its exact prior state; crashes resume without repeating work.
• Read: Step Transaction, journal code, MEGHNA crash tests.
• Do: before each step, record a shadow snapshot (e.g. git stash create or a hidden ref under refs/sdc/shadow/<step>), without touching the user's branch or index; on failure restore it; journal step state (started, verified, done) so a restart skips done steps; writes are keyed by (step, target) so a retry replaces, never appends.
• Don't: create visible commits or change the user's staged files.
• Verify: crash-in-the-middle test; restore test on a dirty tree; git status identical before and after a failed step.
• Switch: nr.git_shadow (off).
• Done when: the crash test shows no repeated cloud call and no half-written file.
N6-3 — Contract lock + Glossary (K21)
• Goal: later steps cannot silently break earlier ones.
• Read: N2-1, N4-1, Blueprint format (N2-3).
• Do: once a symbol's signature is published (first verified step), lock it; a patch that changes a locked signature is rejected unless the step is a contract-change step, which must list and update all call sites found via the index; keep a glossary (names, terms, constants) and add it to every Context Pack.
• Don't: lock private helpers inside a function body.
• Verify: fixtures: signature change rejected; contract-change step updates all call sites.
• Switch: nr.contract_lock (off).
• Done when: no fixture can change a locked signature outside a contract-change step.
N7-1 — Best-of-N with test selection (K22)
• Goal: use cheap local retries before paying for cloud.
• Read: N5-1, N6-2, N1-3.
• Do: trigger only after the normal attempt and two sliced retries fail, or for steps tagged hard; generate N = 2–4 candidates at varied temperature, each from a clean shadow snapshot (sequential unless the budgeter allows 2 slots); run tests on each; pick a passing candidate with the smallest diff; if none passes, hand over to relay with the best candidate's error.
• Don't: run Best-of-N where no automatic test or check exists (no verifier, no selection).
• Verify: fake engine returning one good candidate among bad ones.
• Switch: nr.best_of_n (off).
• Done when: the good candidate is always chosen and bad ones leave no trace.
N7-2 — Tool envelope lock + repeat-call block (K23)
• Goal: valid tool calls without hurting reasoning.
• Read: agent/rescue.rs, M2-1 tool catalogue, Ollama format support.
• Do: constrain only the tool-call envelope (name + arguments schema) with JSON schema; let free-text reasoning and code blocks stay unconstrained; block an identical tool call repeated back-to-back and tell the model the previous result is already in context; if a step requires a tool and the model answers in prose, re-ask once with the tool forced.
• Don't: force JSON for the whole response.
• Verify: fixtures: repeated call blocked, prose-instead-of-tool corrected, malformed args rejected.
• Switch: nr.tool_envelope (off).
• Done when: fixture runs show zero malformed tool calls reaching tools.
N7-3 — Thinking budget (K24)
• Goal: spend thinking tokens only where they help.
• Read: provider settings, step tags.
• Do: if the local model supports a thinking mode, enable it for plan, test-writing and error-fix steps; disable it for boilerplate and formatting steps; respect the window budget (thinking output counts toward reserve).
• Don't: assume a model supports thinking; detect it from model metadata or config.
• Verify: unit tests of the per-tag rule; window reserve accounting test.
• Switch: nr.think_budget (off).
• Done when: boilerplate steps run without thinking and fix steps run with it on a capable fixture model.
Task cards: pillars 8–9
N8-1 — Capability tag + sandwich router (K25)
• Goal: route by step type: architecture to the cloud architect once, implementation to 9B.
• Read: MEGHNA M1-2 RelayDecision, M1-13 preroute, N2-3.
• Do: tag each planned step simple, code, hard or architecture using deterministic rules first (step kind, files touched, Blueprint markers) and the 9B only as tie-breaker; architecture → architect row (only with consent); hard → enables Best-of-N earlier; everything else local. Record the tag and reason in Step Trace.
• Don't: route anything when Manual Pin, local-only, or relay.auto off.
• Verify: routing table tests for every mode combination.
• Switch: nr.tag_route (off).
• Done when: the routing table matches the plan's work-split section for all modes.
N8-2 — Escalation Packet merged with Capsule v2 (K26)
• Goal: the cloud gets exactly what it needs to fix one step.
• Read: MEGHNA M1-3 Capsule v2, N3-3, N4-1, N6-3.
• Do: extend Capsule v2 with: R-IDs and done-when, locked contracts touching the step, skeletons, the 9B's last diff, the sliced error, and the new tests; keep the 6k budget and stable prefix; redact secrets; log to .sdc/audit/sent.jsonl. One structure, not two.
• Don't: send whole files, chat history or other projects.
• Verify: capsule snapshot tests; redaction tests; budget tests.
• Switch: nr.escalation_packet (off; when off, plain Capsule v2 is used).
• Done when: packets stay under budget on all fixtures and contain no secret markers' values.
N8-3 — Role-based rotation + quota failover (K27)
• Goal: the right cloud model for each job, and no stall on quota.
• Read: rotation/mod.rs, ~/.sdc/rotation.toml, M1-4.
• Do: add an optional role per chain row (architect, fixer, any); pick the first healthy row with a matching role, else any; on HTTP 429, quota, missing key or timeout, mark the row cooling-down and move to the next; respect MEGHNA per-turn step and dollar limits.
• Don't: reorder the user's chain; only skip rows.
• Verify: VCR fixtures with 429 and missing-key cases; cost-limit test.
• Switch: nr.role_rotation (off; when off, plain chain order).
• Done when: a 429 on row 1 never stops the turn if row 2 is healthy.
N9-1 — Golden Trace (K28)
• Goal: fewer repeat relays for the same kind of problem.
• Read: MEGHNA M1-12 relay.learn, knowledge_core gate.
• Do: when a cloud fix passes all tests, store (problem signature: error class + symbol kind + language, sliced error, final diff) in a local SQLite table; require the knowledge_core two-witness rule before it can be used; retrieve at most 1 example by signature for future steps; drop an example that is followed by a failure twice.
• Don't: store secrets, redacted values, or anything from local-only projects' secret paths.
• Verify: store/retrieve tests; expiry test; secret-scan test.
• Switch: nr.golden_trace (off).
• Done when: a repeated fixture problem gets its example and the hit is counted in the receipt.
N9-2 — NAVARATNA Bench (K29)
• Goal: every switch is judged by numbers, not claims.
• Read: MEGHNA bench tiers (M0-8, M7-9), all N cards.
• Do: add a navaratna tier: big-prompt tasks with planted traps (mid-prompt requirement, duplicate-prone spec, long output, broken-test temptation, fake package); metrics: local-step share, missing R-IDs, unresolved duplicates, broken writes blocked, first-try patch apply rate, anchor violations, silent truncations, Golden Trace hit rate, wall time; compare each nr.* switch on vs off.
• Don't: run live 9B here (that is N9-3).
• Verify: bench runs end-to-end on the fake engine and writes a report.
• Switch: none.
• Done when: sdcd bench navaratna produces a per-switch on/off table.
N9-3 — Live validation (OWNER-LATER)
• Goal: measure the real 9B.
• Do (owner): with Ollama running, set the chain, run sdcd probe window, then sdcd bench navaratna --live; share the report. The agent then proposes which switches to turn on by default, with numbers.
• Done when: the owner has a report and a written decision per switch.