# Plan Status

Generated from `PROMPT-BOOK-SDC.md` by `sdcd plan sync`; only the status, report and note columns are edited (by `sdcd plan set`).
Status: `todo` · `in-progress` · `blocked` · `done`.

| ID | title | phase | depends_on | status | switch | report | note |
|---|---|---|---|---|---|---|---|
| P0-0 | Plan Tracker | 0 |  | done | n/a | sdc/docs/reports/P0-0.md | plan tracker |
| P0-1 | Characterization tests and baseline | 0 | P0-0 | done | n/a | sdc/docs/reports/P0-1.md | 99.2% 119/120 on deepseek-chat full tier for $0.2023; baseline frozen 2026-10-17 |
| P0-2 | Keychain and Claude compliance | 0 | P0-1 | done | secrets.keychain | sdc/docs/reports/P0-2.md | keys migrate wired + doctor row; Claude CLI credential no longer read |
| P0-3 | Modular split | 0 | P0-1 | done | n/a | sdc/docs/reports/P0-3.md | methods.rs 5682 -> 2130 + 6 domain modules; 83 arms identical, 0 lines lost |
| P0-4 | Crash-resilient journal (local + VPS) | 0 | P0-3 | done | journal.v2 | sdc/docs/reports/P0-4.md | 100/100 local kill -9 + 100/100 runner; versioned records, step_id, origin, fsync, damaged tail counted |
| P0-5 | Data layer | 0 | P0-3 | done | data.v2 | sdc/docs/reports/P0-5.md | rotation.toml + sent.jsonl + capped LRU cache + tested precedence; 28 data tests |
| P0-6 | Daemon security | 0 | P0-3 | done | daemon.secure | sdc/docs/reports/P0-6.md | per-client token, Host/Origin checks, rate limit, no CORS; verified live with curl (200/403 matrix) |
| P0-7 | UI guide and visual baseline | 0 | P0-3 | done | n/a | sdc/docs/reports/P0-7.md | UI-GUIDE (73 tokens) + 4-view pixel visual regression; axe 0 violations |
| P0-8 | Local Runtime Guard | 0 | P0-3 | done | local.runtime_guard | sdc/docs/reports/P0-8.md | count before send, ContextOverflow, prompt_eval_count judged, switch; live Ollama check left to the owner |
| P0-9 | Model identity and capability probe | 0 | P0-8 | done | models.probe | sdc/docs/reports/P0-9.md | identity key + capability probe + class badge (en/bn); 3.0.1: probe runs by itself after a local model's first turn (switch on); live Ollama probe left to the owner |
| P0-10 | Model-selection contract and Pin Guard | 0 | P0-5, P0-9 | done | models.contract (default ON after tests) | sdc/docs/reports/P0-10.md | ModelGate + Pin Guard (7+2 tests), mode badge en/bn, protocol:check fixed for the P0-3 split |
| P0-11 | Bench rules, hard tier and small-model bench | 0 | P0-9 | done | n/a | sdc/docs/reports/P0-11.md | bench rules in code (seed, >=3 runs, noise, summary.json), hard tier + private tasks, 8-task small-model bench validated |
| P0-12 | Speed baseline and Step Trace | 0 | P0-11 | done | trace.steps | sdc/docs/reports/P0-12.md | Step Trace (<1ms/step) + Speed Gate (fails on injected 10%) + Trace tab; baseline median 3.84s p90 5.86s |
| P0-13 | Output compressor, OS normalizer, command runner | 0 | P0-3 | done | tools.compress, tools.osnorm, tools.runner | sdc/docs/reports/P0-13.md | compressor contract (property test), per-OS normaliser, deadline+group-kill runner; 41 guard tests |
| P0-14 | Sandbox Runner and Step Transaction | 0 | P0-13, P0-4 | done | exec.sandbox, edit.transaction | sdc/docs/reports/P0-14.md | Sandbox Runner (macOS proven, Windows gap stated) + Step Transaction (kill at 14 instants, never a mixture) |
| P0-15 | Proof-of-Work badge and receipt | 0 | P0-12 | done | ui.proof_badge | sdc/docs/reports/P0-15.md | Proof-of-Work badge + receipt from the Step Trace (totals equal trace), en/bn, component visual fixture |
| P0-16 | Hardware Auto-Tuner and VRAM Arbiter | 0 | P0-9 | done | local.autotune | sdc/docs/reports/P0-16.md | Auto-Tuner proposal (8GB card -> ~31k ctx on GPU, never applied), spill warnings, VRAM Arbiter |
| P0-17 | Phase 0 release and Gate 0 | 0 | P0-0, P0-1, P0-2, P0-3, P0-4, P0-5, P0-6, P0-7, P0-8, P0-9, P0-10, P0-11, P0-12, P0-13, P0-14, P0-15, P0-16 | done | n/a | sdc/docs/reports/P0-17.md | Gate 0 PASS with measured numbers; audit fixed a shadow-repo bug that could empty a project; v3.0.0 released for macOS/Windows/Linux |
| P1-1 | Code Graph and zero-model queries | 1 | Gate 0 | done | graph.v1 |  | Code Graph on tree-sitter, Merkle index; 8 languages |
| P1-2 | Step Runtime | 1 | P1-1, P0-14, Gate 0 | done | runtime.steps |  | syntax gate + all-or-nothing journaled commit for every edit |
| P1-3 | Small Model Profile and budgeter | 1 | P0-9, Gate 0 | done | sme.budget |  | model classes and one token budget per step |
| P1-4 | Step Profile (sampling and thinking) | 1 | P1-3, Gate 0 | done | sme.step_profile |  | vendor sampling per step; off by default until measured |
| P1-5 | Prompt Diet and stable prefix | 1 | P1-3, Gate 0 | done | sme.prompt_diet |  | tool schemas capped, rules cut to the class, stable prefix |
| P1-6 | Constrained output and Tool-call Rescue | 1 | P1-5, Gate 0 | done | sme.constrained, sme.toolcall_rescue |  | Tool-call Rescue + constrained retry (Ollama format schema); loop-wired test found and fixed a silent turn end |
| P1-7 | Working Memory | 1 | P1-2, Gate 0 | done | sme.memory |  | working-memory ledger (sqlite) + methods |
| P1-8 | Retrieval Cascade and Chunked Reader | 1 | P1-1, P0-16, Gate 0 | done | sme.retrieval |  | retrieval cascade + ranked_search |
| P1-9 | Source Quote Verifier | 1 | P1-7, Gate 0 | done | sme.quote_verify |  | quote verifier wired into note saving |
| P1-10 | Task Decomposer | 1 | P1-2, P1-8, Gate 0 | done | runtime.decompose |  | decomposer wired: several-job requests get a numbered steps note; plan-size check; fast path skips it |
| P1-11 | Symbol-Anchored Patcher and Cut-off Guard | 1 | P1-1, P0-14, Gate 0 | done | edit.symbol_patch |  | symbol patcher + cut-off guard (off by default) |
| P1-12 | LSP Bridge | 1 | P1-1, Gate 0 | done | verify.lsp |  | LSP client + after-edit hook (off by default) |
| P1-13 | Confidence Signals | 1 | P1-2, Gate 0 | done | sme.confidence |  | confidence signals incl. A-B-A-B cycles; level 3 tells the model to stop (does not force it) |
| P1-14 | Bangla/Banglish Spec Brief | 1 | P1-5, Gate 0 | done | sme.spec_brief |  | Bangla/Banglish spec brief |
| P1-15 | Fast path and Gate 1 | 1 | P1-1, P1-2, P1-3, P1-4, P1-5, P1-6, P1-7, P1-8, P1-9, P1-10, P1-11, P1-12, P1-13, P1-14, Gate 0 | in-progress | runtime.fastpath |  | fast path done (runtime.fastpath); Gate 1 not run - needs a live 9B model (owner-to-run) |
| P2-1 | Rotation Policy Engine | 2 | Gate 1 | in-progress | rotation.policy |  | MEGHNA M1 = P2-1/P2-4/P2-5 in the form of Silent Relay (rotation/policy, decision, executor, handover) |
| P2-2 | Rotation Chain Settings UI | 2 | P2-1, P0-5, Gate 1 | done | ui.rotation_chain |  | Settings -> Agents tab (modals/AgentsTab.tsx, rotation.get/set): the chain, order and manual Escalate; MEGHNA M1 builds on it |
| P2-3 | Capsule Minimizer, Redactor, audit | 2 | P2-1, Gate 1 | done | rotation.capsule |  | capsule builder with redaction + .sdc/audit/sent.jsonl (sdcp/rotation_methods.rs build_capsule, v3/privacy.rs); MEGHNA M1-3 extends it to Capsule v2 |
| P2-4 | Chain Escalation Runner | 2 | P2-3, Gate 1 | todo | rotation.runner |  |  |
| P2-5 | Handover Bridge Note | 2 | P2-4, P1-9, Gate 1 | todo | rotation.handover |  |  |
| P2-6 | Planner-Executor | 2 | P2-4, Gate 1 | todo | rotation.planner_exec |  |  |
| P2-7 | Chain-to-local learning and Failure Taxonomy | 2 | P2-5, Gate 1 | todo | learn.chain_recipes |  |  |
| P2-8 | Rotation safeguards and Gate 2 | 2 | P2-1, P2-2, P2-3, P2-4, P2-5, P2-6, P2-7, Gate 1 | todo | rotation.safeguards |  |  |
| P3-1 | Reuse Finder | 3 | Gate 2 | todo | precision.reuse |  |  |
| P3-2 | Diff Budget | 3 | P3-1, Gate 2 | todo | precision.diff_budget |  |  |
| P3-3 | Hallucination and Slopsquatting Guard | 3 | P1-1, Gate 2 | done | precision.hallucination |  | trust/slopguard + trust.slopguard switch (on): an install of a name the registry does not know is refused |
| P3-4 | Test Impact, Characterization, Smoke | 3 | P1-1, P0-14, Gate 2 | todo | verify.test_impact |  |  |
| P3-5 | Quality Floor and AST Duplicates | 3 | P1-1, Gate 2 | todo | precision.quality_floor |  |  |
| P3-6 | Ripple Check | 3 | P1-12, Gate 2 | todo | precision.ripple |  |  |
| P3-7 | Spec-to-Test | 3 | P3-4, Gate 2 | todo | precision.spec_tests |  |  |
| P3-8 | Q3 profiling for all models | 3 | P0-11, Gate 2 | todo | n/a |  |  |
| P3-9 | Diff Review UI | 3 | P0-7, Gate 2 | done | ui.diff_review |  | panels/right/PreviewDiff.tsx + lib/diffReview.ts (+test): per-hunk review in the Preview |
| P3-10 | @ Context Picker | 3 | P1-1, Gate 2 | todo | ui.at_picker |  |  |
| P3-11 | Select & Instruct | 3 | P3-9, Gate 2 | done | ui.select_instruct |  | panels/right/SelectInstruct.tsx: select code in the Preview and instruct the agent about it |
| P3-12 | Local Semantic Index | 3 | P1-8, Gate 2 | todo | graph.semantic |  |  |
| P3-13 | Scoped Rules, Hooks, rule import | 3 | P1-5, Gate 2 | todo | rules.scoped |  |  |
| P3-14 | Verified MCP Installer | 3 | P0-6, P0-2, Gate 2 | todo | mcp.installer |  |  |
| P3-15 | Local GPU Scheduler and Gate 3 | 3 | P3-1, P3-2, P3-3, P3-4, P3-5, P3-6, P3-7, P3-8, P3-9, P3-10, P3-11, P3-12, P3-13, P3-14, Gate 2 | todo | local.scheduler |  |  |
| P4-1 | Replay Lab | 4 | Gate 3 | todo | learn.replay |  |  |
| P4-2 | Smart Router | 4 | P4-1, Gate 3 | todo | rotation.smart_router |  |  |
| P4-3 | Night Learning, Drift Monitor, weekly report | 4 | P4-1, Gate 3 | todo | learn.night |  |  |
| P4-4 | SDC as MCP Server | 4 | P0-6, Gate 3 | done | mcp.server |  | sdcd mcp (mcp.rs, docs/MCP.md): policy, trace and memory tools for Claude Code/Codex; MEGHNA M5-4 extends it |
| P4-5 | VS Code Companion | 4 | P4-4, Gate 3 | todo | ext.vscode |  |  |
| P5-1 | Continuity Sync | 5 | Gate 4 | todo | sync.continuity |  |  |
| P5-2 | DevOps Dry-run and Playbook Verifier | 5 | P5-1, Gate 4 | todo | ops.dryrun |  |  |
| P5-3 | Background Agent on the user's VPS | 5 | P5-1, Gate 4 | todo | agent.background |  |  |
| P5-4 | Self-hosted PR Review Bot | 5 | P5-3, Gate 4 | todo | agent.pr_review |  |  |
| P5-5 | GitHub Issues, Telegram, Slack | 5 | P5-3, Gate 4 | todo | integrations.messaging |  |  |
