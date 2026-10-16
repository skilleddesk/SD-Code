# Plan Status

Generated from `PROMPT-BOOK-SDC.md` by `sdcd plan sync`; only the status, report and note columns are edited (by `sdcd plan set`).
Status: `todo` · `in-progress` · `blocked` · `done`.

| ID | title | phase | depends_on | status | switch | report | note |
|---|---|---|---|---|---|---|---|
| P0-0 | Plan Tracker | 0 |  | done | n/a | sdc/docs/reports/P0-0.md | plan tracker |
| P0-1 | Characterization tests and baseline | 0 | P0-0 | todo | n/a |  |  |
| P0-2 | Keychain and Claude compliance | 0 | P0-1 | todo | secrets.keychain |  |  |
| P0-3 | Modular split | 0 | P0-1 | todo | n/a |  |  |
| P0-4 | Crash-resilient journal (local + VPS) | 0 | P0-3 | todo | journal.v2 |  |  |
| P0-5 | Data layer | 0 | P0-3 | todo | data.v2 |  |  |
| P0-6 | Daemon security | 0 | P0-3 | todo | daemon.secure |  |  |
| P0-7 | UI guide and visual baseline | 0 | P0-3 | todo | n/a |  |  |
| P0-8 | Local Runtime Guard | 0 | P0-3 | todo | local.runtime_guard |  |  |
| P0-9 | Model identity and capability probe | 0 | P0-8 | todo | models.probe |  |  |
| P0-10 | Model-selection contract and Pin Guard | 0 | P0-5, P0-9 | todo | models.contract (default ON after tests) |  |  |
| P0-11 | Bench rules, hard tier and small-model bench | 0 | P0-9 | todo | n/a |  |  |
| P0-12 | Speed baseline and Step Trace | 0 | P0-11 | todo | trace.steps |  |  |
| P0-13 | Output compressor, OS normalizer, command runner | 0 | P0-3 | todo | tools.compress, tools.osnorm, tools.runner |  |  |
| P0-14 | Sandbox Runner and Step Transaction | 0 | P0-13, P0-4 | todo | exec.sandbox, edit.transaction |  |  |
| P0-15 | Proof-of-Work badge and receipt | 0 | P0-12 | todo | ui.proof_badge |  |  |
| P0-16 | Hardware Auto-Tuner and VRAM Arbiter | 0 | P0-9 | todo | local.autotune |  |  |
| P0-17 | Phase 0 release and Gate 0 | 0 | P0-0, P0-1, P0-2, P0-3, P0-4, P0-5, P0-6, P0-7, P0-8, P0-9, P0-10, P0-11, P0-12, P0-13, P0-14, P0-15, P0-16 | todo | n/a |  |  |
| P1-1 | Code Graph and zero-model queries | 1 | Gate 0 | todo | graph.v1 |  |  |
| P1-2 | Step Runtime | 1 | P1-1, P0-14, Gate 0 | todo | runtime.steps |  |  |
| P1-3 | Small Model Profile and budgeter | 1 | P0-9, Gate 0 | todo | sme.budget |  |  |
| P1-4 | Step Profile (sampling and thinking) | 1 | P1-3, Gate 0 | todo | sme.step_profile |  |  |
| P1-5 | Prompt Diet and stable prefix | 1 | P1-3, Gate 0 | todo | sme.prompt_diet |  |  |
| P1-6 | Constrained output and Tool-call Rescue | 1 | P1-5, Gate 0 | todo | sme.constrained, sme.toolcall_rescue |  |  |
| P1-7 | Working Memory | 1 | P1-2, Gate 0 | todo | sme.memory |  |  |
| P1-8 | Retrieval Cascade and Chunked Reader | 1 | P1-1, P0-16, Gate 0 | todo | sme.retrieval |  |  |
| P1-9 | Source Quote Verifier | 1 | P1-7, Gate 0 | todo | sme.quote_verify |  |  |
| P1-10 | Task Decomposer | 1 | P1-2, P1-8, Gate 0 | todo | runtime.decompose |  |  |
| P1-11 | Symbol-Anchored Patcher and Cut-off Guard | 1 | P1-1, P0-14, Gate 0 | todo | edit.symbol_patch |  |  |
| P1-12 | LSP Bridge | 1 | P1-1, Gate 0 | todo | verify.lsp |  |  |
| P1-13 | Confidence Signals | 1 | P1-2, Gate 0 | todo | sme.confidence |  |  |
| P1-14 | Bangla/Banglish Spec Brief | 1 | P1-5, Gate 0 | todo | sme.spec_brief |  |  |
| P1-15 | Fast path and Gate 1 | 1 | P1-1, P1-2, P1-3, P1-4, P1-5, P1-6, P1-7, P1-8, P1-9, P1-10, P1-11, P1-12, P1-13, P1-14, Gate 0 | todo | runtime.fastpath |  |  |
| P2-1 | Rotation Policy Engine | 2 | Gate 1 | todo | rotation.policy |  |  |
| P2-2 | Rotation Chain Settings UI | 2 | P2-1, P0-5, Gate 1 | todo | ui.rotation_chain |  |  |
| P2-3 | Capsule Minimizer, Redactor, audit | 2 | P2-1, Gate 1 | todo | rotation.capsule |  |  |
| P2-4 | Chain Escalation Runner | 2 | P2-3, Gate 1 | todo | rotation.runner |  |  |
| P2-5 | Handover Bridge Note | 2 | P2-4, P1-9, Gate 1 | todo | rotation.handover |  |  |
| P2-6 | Planner-Executor | 2 | P2-4, Gate 1 | todo | rotation.planner_exec |  |  |
| P2-7 | Chain-to-local learning and Failure Taxonomy | 2 | P2-5, Gate 1 | todo | learn.chain_recipes |  |  |
| P2-8 | Rotation safeguards and Gate 2 | 2 | P2-1, P2-2, P2-3, P2-4, P2-5, P2-6, P2-7, Gate 1 | todo | rotation.safeguards |  |  |
| P3-1 | Reuse Finder | 3 | Gate 2 | todo | precision.reuse |  |  |
| P3-2 | Diff Budget | 3 | P3-1, Gate 2 | todo | precision.diff_budget |  |  |
| P3-3 | Hallucination and Slopsquatting Guard | 3 | P1-1, Gate 2 | todo | precision.hallucination |  |  |
| P3-4 | Test Impact, Characterization, Smoke | 3 | P1-1, P0-14, Gate 2 | todo | verify.test_impact |  |  |
| P3-5 | Quality Floor and AST Duplicates | 3 | P1-1, Gate 2 | todo | precision.quality_floor |  |  |
| P3-6 | Ripple Check | 3 | P1-12, Gate 2 | todo | precision.ripple |  |  |
| P3-7 | Spec-to-Test | 3 | P3-4, Gate 2 | todo | precision.spec_tests |  |  |
| P3-8 | Q3 profiling for all models | 3 | P0-11, Gate 2 | todo | n/a |  |  |
| P3-9 | Diff Review UI | 3 | P0-7, Gate 2 | todo | ui.diff_review |  |  |
| P3-10 | @ Context Picker | 3 | P1-1, Gate 2 | todo | ui.at_picker |  |  |
| P3-11 | Select & Instruct | 3 | P3-9, Gate 2 | todo | ui.select_instruct |  |  |
| P3-12 | Local Semantic Index | 3 | P1-8, Gate 2 | todo | graph.semantic |  |  |
| P3-13 | Scoped Rules, Hooks, rule import | 3 | P1-5, Gate 2 | todo | rules.scoped |  |  |
| P3-14 | Verified MCP Installer | 3 | P0-6, P0-2, Gate 2 | todo | mcp.installer |  |  |
| P3-15 | Local GPU Scheduler and Gate 3 | 3 | P3-1, P3-2, P3-3, P3-4, P3-5, P3-6, P3-7, P3-8, P3-9, P3-10, P3-11, P3-12, P3-13, P3-14, Gate 2 | todo | local.scheduler |  |  |
| P4-1 | Replay Lab | 4 | Gate 3 | todo | learn.replay |  |  |
| P4-2 | Smart Router | 4 | P4-1, Gate 3 | todo | rotation.smart_router |  |  |
| P4-3 | Night Learning, Drift Monitor, weekly report | 4 | P4-1, Gate 3 | todo | learn.night |  |  |
| P4-4 | SDC as MCP Server | 4 | P0-6, Gate 3 | todo | mcp.server |  |  |
| P4-5 | VS Code Companion | 4 | P4-4, Gate 3 | todo | ext.vscode |  |  |
| P5-1 | Continuity Sync | 5 | Gate 4 | todo | sync.continuity |  |  |
| P5-2 | DevOps Dry-run and Playbook Verifier | 5 | P5-1, Gate 4 | todo | ops.dryrun |  |  |
| P5-3 | Background Agent on the user's VPS | 5 | P5-1, Gate 4 | todo | agent.background |  |  |
| P5-4 | Self-hosted PR Review Bot | 5 | P5-3, Gate 4 | todo | agent.pr_review |  |  |
| P5-5 | GitHub Issues, Telegram, Slack | 5 | P5-3, Gate 4 | todo | integrations.messaging |  |  |
