PADMA-PROMPTS — Task cards for Claude Code
Oct 25, 2026 · @Mehedi Hasan
How to use
These are the English task cards for the PADMA plan (PADMA.md, Bengali). Paste a card into Claude Code as-is, or just write PADMA next / PADMA P0-3. The plan file defines triggers, rules D1–D7, root causes RC1–RC10 and guarantees G1–G10.
Kickoff prompt (use once, before P0-1):
PADMA. Read PADMA.md and PADMA-PROMPTS.md (same skills folder) completely, then
MEGHNA.md and NAVARATNA.md for inherited rules R1–R11 and N1–N7.
Run `git status` (stop if dirty). Map the current agent loop: where a turn ends,
where tool calls are parsed from the Ollama response, where ContextOverflow is
raised, where UI notices (GPU %) are appended, where Trust is computed, and which
nr.* switches gate the NAVARATNA step-executor. Write the map to
docs/padma/CURRENT-LOOP.md with file:line references. No behavior changes.
Report in Bengali using the PADMA report format.
Global rules for every card:
1. One card per turn. Read every file before editing it.
2. Every new behavior sits behind a padma.* switch with the default the card states (D1). Register the switch in the existing settings/switch registry with an honest one-line description.
3. New protocol methods/events go in protocol/sdcp.schema.json, protocol/types.ts and daemon dispatch; node protocol/check.mjs must pass (R5).
4. No live Ollama in tests. Use the scripted fake engine and VCR fixtures (R8). Live checks are OWNER-LATER.
5. The Never-Again suite (tests/never_again/) must stay green after P0-1 turns it green test by test. A card that turns any of them red is not done (D2).
6. Never hardcode a model name in Conductor or Recipe code (D5). Model quirks live in Model Profiles and the Adapter.
7. Cross-platform: Windows, macOS, Linux. Paths with spaces (e.g. F:\SDC Code\) must work everywhere.
8. Do not print or store secrets (R2). Revert or separately commit Cargo.lock churn (R9).
9. Commit: padma(<ID>): <English summary>. Update the PADMA.md tracker row (status + commit) and CHANGELOG Unreleased.
10. Report in Bengali; code, identifiers, commands and commit messages in English.
Phase P0 — Never-Again fixes
P0-1 — Never-Again fixture suite (G1–G10)
• Goal: Lock the 25 Oct 2026 incident into tests that fail today and pass only when fixed.
• Do: Create tests/never_again/ with VCR fixtures that replay the fake-engine outputs observed: (a) reply ending in : with no tool call ("now checking build:"); (b) empty content with only thinking; (c) a tool call printed as text <tool_call>…</tool_call> inside content; (d) 25 tool steps whose accumulated observations push the request to 12,397 + 4,096 reserve > 16,384; (e) user pastes the error text as the next message; (f) user sends "ok koro" after a failure; (g) read_file failing with Windows os error 2 and 3 on a path containing a space; (h) a GPU-offload notice emitted during streaming. Write one test per guarantee: announce_then_stop, colon_end_no_tool, qwen35_text_toolcall, thinking_only, empty_answer, overflow_by_109, short_prompt_full_review, resume_after_fail, win_os_error_2_3_5, no_evidence_claim, no_ui_notice_in_history.
• Done when: All tests exist, run in CI, and are marked expected-fail with a reason pointing to the card that fixes each one. Each later card removes its expected-fail marker.
• Verify: Test runner output listing every test as xfail.
P0-2 — Ollama client + tolerant tool-call parser (G3)
• Goal: No tool call is ever lost because of format.
• Do: In the Ollama client, read message.content, message.thinking and message.tool_calls separately (native /api/chat; for the OpenAI-compatible path also read the reasoning field). Add ToolCallExtractor that tries, in order: native tool_calls; <tool_call> blocks in content (JSON body or Qwen3-Coder XML <function=…><parameter=…>); the same inside thinking; one fenced JSON object matching a known tool. Validate against the tool schema; on failure return a short schema error to the model (reuse N7-2 envelope). Never send prior-turn thinking back in history; always close any unclosed <think> before re-sending.
• Switch: padma.tolerant_parse (on).
• Done when: qwen35_text_toolcall and thinking_only pass; existing tool tests unchanged.
P0-3 — Intent-without-Action guard (G2)
• Goal: "I'll do X:" followed by silence is never accepted as a final answer.
• Do: After each model reply with zero tool calls, run IntentDetector when the active Task Contract is not finished (before P1-1 exists: when the turn has a plan with unfinished steps or the reply is under 300 tokens). Signals: last non-space char is :; future-intent phrases (multilingual list in intent_phrases.toml: "I'll", "let me", "now checking", "দেখছি", "করছি", "শুরু করছি", "ধাপ ১" without "ধাপ ২"); an opened numbered list with one item. Score ≥ 2 signals, or the colon signal alone → re-prompt with: You described an action but did not call a tool. Call the tool now, or reply FINAL: <answer> if the task is complete. Max 2 re-prompts, then hand to Recovery Ladder (P0-4).
• Switch: padma.intent_guard (on).
• Done when: announce_then_stop and colon_end_no_tool pass; a genuine short answer ("2 + 2 = 4") is not re-prompted (add that negative test).
P0-4 — Recovery Ladder (G4)
• Goal: An empty or contentless reply never ends the task.
• Do: Implement RecoveryLadder with rungs: (1) same request + one-line nudge; (2) thinking off; (3) tighter context (masking k=1, smaller unit); (4) Best-of-N via N7-1 if enabled; (5) next row of the rotation chain when relay.auto is on (respect cost limits and privacy); (6) mark the step blocked(reason) and continue with independent steps. Log each rung as a step-trace event with reason.
• Switch: padma.recovery (on).
• Done when: empty_answer passes; the ladder never loops more than 6 rungs per step.
P0-5 — Overflow Rule + obs_cap + Observation Masking (G5)
• Goal: ContextOverflow is never a terminal failure.
• Do: (a) obs_cap = 10% of T_in per tool output; overflow goes to the vault as ‹obs:N›, with a new obs_read(n, range) tool. (b) Observation masking: keep the last k=3 tool outputs whole; replace older ones with a one-line stub [tool args → ‹obs:N›, note]. (c) Before every model call compute the request size; if it exceeds the window apply in order: k→1; Context Pack drop order; output_reserve 4096→1536 for tool-call steps; split the unit; else mark the unit blocked(too_large) and move on. Add an explicit rule so the error text "This failure has no rule yet" can no longer appear for ContextOverflow.
• Switches: padma.overflow_rule, padma.obs_cap (on).
• Done when: overflow_by_109 passes and the replayed 25-step run continues past step 25.
P0-6 — Clean history (G10)
• Goal: UI notices never reach the model.
• Do: Move the GPU-offload notice, slow-model warnings, Trust badges and receipts to separate UI events. Assert in the history builder that assistant messages contain only model output. Remove the duplicated notice in streaming.
• Switch: padma.clean_history (on).
• Done when: no_ui_notice_in_history passes; the notice still shows in the UI as a badge.
P0-7 — Evidence-based Trust (G9)
• Goal: Trust reflects proof, not tone.
• Do: Trust inputs: tool calls executed, gates passed, findings with matching evidence lines, intent-guard hits, recovery rungs used. A reply that claims an action with zero tool evidence caps at "low"; a turn ending blocked shows "blocked", not "high".
• Switch: padma.evidence_trust (on).
• Done when: no_evidence_claim passes.
P0-8 — Tool error hints (G8)
• Goal: The model gets the true cause of a file error, never a guess.
• Do: Map OS errors to plain meanings for all three OSes (Windows 2 file not found, 3 path not found, 5 access denied, 32 file in use; POSIX ENOENT, EACCES, EPERM, EBUSY). Return: meaning, resolved absolute path, current working directory, up to 3 nearest existing paths by fuzzy match. Always quote paths with spaces in run_command. If a read targets a path never listed this task, suggest list_dir first.
• Switch: padma.tool_hints (on).
• Done when: win_os_error_2_3_5 passes on the Windows fixture; POSIX equivalents covered.
Phase P1 — Conductor, Ledger, Finish Gate
P1-1 — Task Contract + Task Ledger (G7)
• Goal: Every task is written down and survives failures, restarts and new messages.
• Do: Define TaskContract { id, goal_en, original_ref (vault), recipe, scope, done_when[], deliverable, language_out, created_at } and TaskLedger (SQLite under the project's .sdc/, reuse M7-2 vault/ledger storage where it exists). Each step row: step_id, unit, status (todo/running/done/blocked), attempts, evidence_refs, handover_note. Write on every transition (journal-style, crash-safe). Expose protocol task.get, task.list, events task.progress.
• Switch: padma.ledger (on, bookkeeping only).
• Done when: Killing the daemon mid-run and restarting shows the same step states; unit test for crash-resume.
P1-2 — Finish Gate (G1)
• Goal: Only code decides that a turn is finished.
• Do: A turn with an active contract can end only when (a) every done_when has evidence in the ledger, or (b) remaining steps are blocked with reasons, or (c) the phase budget is spent and progress is checkpointed with a clear "continue" message, or (d) the user cancels. A model reply that says it is done but fails (a)–(d) goes back to the Conductor as a signal. The final message always lists done / blocked / remaining counts.
• Switch: padma.finish_gate (on).
• Done when: announce_then_stop passes end-to-end (intent guard + finish gate); a normal one-shot Q&A still ends in one step.
P1-3 — Scope Classifier (G6)
• Goal: A short prompt for a big job triggers the right Recipe.
• Do: Code-first classifier, no LLM unless ambiguous. Inputs: normalized intent (verbs like review/audit/check all/find errors/build/make/fix, in English, Bengali and Banglish: "sompurno", "puro", "sob", "review koro", "bug khujo"), target (repo, folder, URL, VPS path), repo stats (file count, LOC from the symbol index). Output: recipe (R-REVIEW-CODE, R-REVIEW-SITE, R-BUILD-PAGE, R-FIX, R-ANSWER) + size (small/large). Large if the target set exceeds one Context Pack. If ambiguous, one tiny 9B call with a fixed label list (no free text). Replace the "prompt ≥ 35% of T_in" rule as the only Brick/NAVARATNA trigger: either condition now activates the step executor.
• Switch: padma.scope (on, tagging only; routing happens when padma.recipes is on).
• Done when: short_prompt_full_review passes; a 20-case labelled set (EN/BN/Banglish) reaches ≥ 90% accuracy.
P1-4 — Conductor state machine + Phase Budget
• Goal: One driver for every multi-step task.
• Do: States: classify → contract → plan → [budget → model → gate → ledger]* → finish → report. Replace the global "25 steps per turn" cap with a per-step budget (e.g. ≤ 8 tool calls per unit) plus a per-turn wall-clock budget; when the turn budget ends, checkpoint and emit "continue". Emit task.progress ("file 37/120"). Integrate P0-3/P0-4/P0-5 hooks.
• Switch: padma.conductor (off until bench).
• Done when: The replayed incident completes all units in the fake engine; Speed Gate ≤ 5% on small Q&A.
P1-5 — Continuation binding (G7)
• Goal: "ok koro", "continue", "go on", "keep going" or a pasted SDC error resume the active task.
• Do: Before classification, check for an active unfinished contract in this session. Bind the message to it when it is a continuation phrase (multilingual list), a pasted SDC error string (match known error prefixes like ContextOverflow: and ollama failed), or a short question about that failure (then answer from ledger facts and offer to continue). Otherwise start a new contract.
• Switch: padma.continue_bind (on).
• Done when: resume_after_fail passes; an unrelated new request still creates a new contract.
Phase P2 — Model Adapter
P2-1 — Model Profile + Capability Probe
• Goal: Any small model works through one adapter; its quirks are measured, not guessed.
• Do: ~/.sdc/models/<model>.toml with: tool_format (native | hermes-json | qwen3-coder-xml), thinking per step kind, sampling (temperature, top_p, top_k, presence_penalty), effective_ctx (from N1-1 probe), quant, max_tools_per_call. On first use (and on sdcd models probe <name>), run a 2-minute probe against the live model: 5 single-tool tasks, 3 no-tool questions, 2 multi-step tasks; record tool-call success rate, text-form tool-call rate, empty-reply rate, best tool_format. Show a one-line summary in Settings → Models. Probe is OWNER-LATER for live runs; implement with VCR in tests.
• Switch: padma.model_profile (on, read-only use).
• Done when: Profiles load for qwen3.5:9b and one other local model fixture; Conductor code contains no model names.
P2-2 — Thinking policy per step kind
• Do: Step kinds: tool_call, map_review, boilerplate → thinking off; blueprint, repair_after_fail, reduce_report → thinking on with N7-3 budget. Pass the model's native switch (Ollama think flag or the profile's method).
• Switch: padma.think_policy (off until bench).
• Done when: Fake-engine test shows the right flag per step; empty-reply rate on the VCR set does not rise.
P2-3 — Per-step tool subset
• Do: Each Recipe step declares 2–4 tools (e.g. map_review: read_range, obs_read, report_finding). Only those schemas go into the prompt. Reuse M2-1 catalog if present.
• Switch: padma.tool_subset (off until bench).
• Done when: Prompt size per step drops measurably in the bench receipt; no tool-not-found regressions.
P2-4 — English inside, user's language outside
• Do: Extend N2-2 so every Task Contract stores goal_en (translation by the 9B in a tiny fixed-format call, original kept in vault). All inner prompts, notes and findings are English. Only reduce_report and user-facing status lines use language_out (Bengali here).
• Switch: padma.inner_english (off until bench).
• Done when: Bench shows fewer tokens per step with equal or better completion; the final report is in Bengali.
Phase P3–P4 — Recipes, integration, bench
P3-1 — Recipe Engine
• Goal: Workflows are data, not ad-hoc loops.
• Do: Load recipes/*.toml. A recipe is a DAG of steps, each kind = code | model | gate, with inputs, outputs, tool subset, budget and done_when. code steps call Rust/TS functions by name; model steps go through Context Budget → Model Adapter; gate steps return pass/fail + evidence. Fan-out (for_each unit) is native so map-reduce needs no special code.
• Switch: padma.recipes (off until bench).
• Done when: A toy recipe (list files → count lines → report) runs on the fake engine with ledger rows per unit.
P3-2 — R-REVIEW-CODE + Findings Ledger + evidence gate
• Do: Steps exactly as PADMA.md: inventory (respect .gitignore; skip node_modules, target, dist); collect real errors by detected stack (cargo check, cargo clippy, tsc --noEmit, eslint, project test command) with Error Slicer; risk order (recent changes, size, errors, fan-in; UI first if asked); per-unit map review (≤ 50% T_in, fixed output line file:line · severity · category · problem · evidence); evidence gate (evidence line must exist verbatim, else drop); dedupe + rank; reduce to report in language_out, chunked via N4-4; finish gate lists any unit not reviewed.
• Done when: On the SDC repo fixture (a pinned snapshot) the fake engine produces a complete report with zero unmatched-evidence findings; progress events show every unit.
P3-3 — R-REVIEW-SITE
• Do: Target detection (local folder / VPS path via existing SDC Anywhere connection / live URL). Code probes: HTTP status and redirects, broken internal links, missing title/meta/alt, oversized images, mixed content, TLS expiry; on VPS nginx -t or apachectl configtest, recent error-log tail. Then hand source folders to R-REVIEW-CODE. Missing access → skipped (no access), never a stop.
• Switch: padma.site_probe (off).
• Done when: Fixture site with 3 planted problems yields all 3 in the report.
P3-4 — R-BUILD-PAGE
• Do: Blueprint (N2-3; cloud only with consent) → coverage and contract check → one fresh step per file (N4-2 symbol patch or new file) → parse/lint/build/HTML-validate gates → Git Shadow per step → finish when every Blueprint file exists and build is green. Unstated choices get sensible defaults written into the report (D6), no upfront questions.
• Done when: Prompt "make a landing page for a bakery" completes on the fake engine with all files present and validator green.
P3-5 — R-FIX (Agentless-style)
• Do: reproduce (code) → hierarchical localization (file → symbol → line) → patch (9B; Best-of-N N7-1 on failure) → test selection (N5-1) → regression → finish.
• Done when: 3 fixture bugs fixed with tests green; failing case ends blocked with reason, not silence.
P3-6 — UI: Recipe chip, progress card, Findings view
• Do: Extend the current SDC UI style (no new design language). Chip shows the recipe; progress card shows unit n/m, findings count, local/cloud split; Findings view sorts by severity with file links. All strings in strings.ts + locales/bn.
• Done when: Screenshot test in light and dark themes.
P4-1 — Wire NAVARATNA engines into Recipes + relay
• Do: Inside Recipe model steps always use fresh step (N3-1), skeleton pack (N3-2), symbol patch (N4-2), test-first where tagged (N5-1), registry (N6-1), Git Shadow (N6-2), regardless of prompt length. Capability failures after the Recovery Ladder go to Silent Relay (M1-4) when consented.
• Done when: Bench receipt shows these engines active on a two-line prompt.
P4-2 — PADMA Bench tier: short prompt, big task
• Do: At least 10 tasks with two-line prompts in EN, BN and Banglish: review whole repo, find UI errors, review website, build landing page, fix failing test, etc. Metrics: completion rate, stops without tool, overflow deaths, empty-reply stops, unmatched findings, local share, time. Compare padma.* on vs off.
• Done when: Bench runs on fake engine + VCR; live run is P4-3 (OWNER-LATER).
P4-3 — Live verification (OWNER-LATER)
• Owner runs the bench with Ollama + qwen3.5:9b, then repeats the original prompt "পুরো SDC প্রজেক্ট দেখো, error রিপোর্ট করো". Decide which off-by-default switches turn on, based only on measured results.