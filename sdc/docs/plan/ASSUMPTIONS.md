# SDC-v3 — Assumption register

Every assumption the plan rests on, with its current status and the evidence behind it.
Status: `untested` · `proven` · `partial` · `false`. Evidence links point to code (`file:line`), to
`bench/results/<run>/` or to a report in `docs/reports/`. Update the row whenever data arrives; never
change a status without a link.

| ID | Assumption | Status | Evidence | Measured in |
|---|---|---|---|---|
| A-01 | Code facts from the analysis are correct (see A-01 detail below) | partial | 4 true, 3 partial, 2 false — below | Phase 0 |
| A-02 | Input shares: ~25% system + tools, ~50% tool results | untested | — | W1 ledger |
| A-03 | Token variance CV ≤ 15% across repeated runs | untested | — | W1 |
| A-04 | Root cause of slow search identified | proven | The v1 walk was the cost: every query 227-261 ms whatever it matched, and a no-match scan 1.2 s. Replacing the walk with a watched index alone took matching queries to 2-4 ms; the remaining full-scan cost was file reads (ripgrep needs the same ~0.63 s), removed by a trigram content index → 6.6 ms (`results/2026-10-14-search-v1` vs `-v3`, same 52,501-file tree, macOS arm64, warm cache) | W1 `bench search`, W3 |
| A-05 | Search p50 < 50 ms filename / < 200 ms text on 50k files | proven (macOS) | v3 p50: filename 2.5-3.2 ms, text 2.0-6.6 ms incl. no-match; v1 229-234 / 227-1204 ms (`results/2026-10-14-search-v3`). Warm OS cache; the first call (index build) is 150 ms. Windows and Linux to be measured in CI | W3 |
| A-06 | Repo map cuts find-task input ≥ 30% on all models | untested | — | W4 |
| A-07 | Tool pruning keeps quality (Q3 rule) | untested | — | W5 |
| A-08 | Cached share ≥ 70% after 3 turns, per provider | untested | — | W5 |
| A-09 | Claude Code / Codex CLI already use prompt caching | proven for Claude Code | input-এর 87% (DeepSeek-এ) ও 86% (Haiku-তে) provider-এর cache থেকে — `bench/results/vs-cc-on-deepseek`, `vs-cc-on-claude-haiku`; Codex মাপা হয়নি (login নেই) | W1 `bench competitors` |
| A-10 | Duplicate reads ≥ 10% of input on long tasks | untested | — | W1 ledger |
| A-11 | Skeleton reads −20% read tokens, time < +20% | untested | — | I2 |
| A-12 | Cheap-API draft acceptance ≥ 35% | untested | — | I3 |
| A-13 | Recipes −50% tool calls on repeated tasks | untested | — | I4 |
| A-14 | Bengali costs 2–4× English tokens, per provider | untested | — | W1 |
| A-15 | Interlingua keeps meaning ≥ 90% | untested | — | I8 |
| A-16 | Fleet clustering −80% at 20 servers, 0 hidden diffs | partial (synthetic) | synthetic fleets: 5 → −44.6%, 10 → −68.3%, 20 → −80.4%, 0 hidden diffs each (SDC's token count, `results/2026-10-14-fleet-synthetic`); a real Docker fleet and provider counts are still to come | I7 |
| A-17 | Self-learning causes no lasting regression over 4 weeks | untested | — | I11 / L |
| A-18 | Knowledge gate blocks ≥ 98% of 50 poisoned items, rest rolled back | holds (synthetic) | 50 poisoned proposals (instructions dressed as knowledge, the same claim the other way round, one sighting only, too short, a key hidden in a lesson) → 50/50 blocked, 0 rolled back needed; a rollback does take an admitted item back (test `fifty_poisoned_proposals_are_blocked_at_the_door`) | I11 |
| A-19 | ≥ 95% correct resume after forced kill, 0 data loss | partial (local proven) | local: 100 SIGKILLs at random moments → 100/100 correct, 0 damaged files, 0 duplicated commands, scripted provider (`bench/results/2026-10-14-resume`, `sdcd bench resume`). Remote (VPS) part waits for R4 | R1–R2 (local), R4 (remote) |
| A-20 | Sync server holds no plaintext | untested | — | X3 canary scan |
| A-21 | Identical behaviour on 3 OS | untested | — | CI + X4 handoff |
| A-22 | v3 cloud bill ≥ 40% below Claude Code / Codex on the same model | partial | DeepSeek-এ Claude Code-এর চেয়ে 65% কম ($0.0324 → $0.0114), 6/6 জোড়ায় SDC সস্তা, জোড়া-প্রতি median 72% [49–75]; pass 6/6 দুদিকে। ৩ task × ২ run, ছোট fixture, এক model; Codex ও অন্য model বাকি — `docs/reports/vs-claude-code-bn.md` | Phase 7 |

## A-01 detail — code facts (checked against `sdc-baseline-v1` = `274ec6a`)

Paths are relative to `sdc/`. Checked 2026-10-14.

| # | Claim | Verdict | Evidence / actual value |
|---|---|---|---|
| a | `DEFAULT_STEPS = usize::MAX` | true | `sdcd/src/agent/mod.rs:58`. Other limits still apply: caller `maxSteps` (`sdcp/methods.rs:1617`), research turns = searches + pages + 8 (`agent/research.rs:44`), sub-agents `SUB_STEPS = 24` (`agent/mod.rs:741`), runaway detector stops the 5th identical call (`trust/cost.rs:331`), budget check before a turn (`sdcp/methods.rs:1600`) |
| b | `PAGE_CAP = 60_000` | true | `sdcd/src/agent/web.rs:13`; local models use `result_cap - 400`, min 2,000 (`agent/tools.rs:1244`) |
| c | `RESULT_CAP = 24_000` | true | `sdcd/src/agent/tools.rs:38`; on Ollama clamped to [2,000, window] (`agent/mod.rs:1597`) |
| d | DuckDuckGo HTML, 8 results | partial | `agent/web.rs:225`; 8 for API models, 5 for local (`agent/tools.rs:1283`). DuckDuckGo is only the default: SearXNG/Tavily/Brave/Serper already exist (`agent/research.rs:50-80`) |
| e | `privacy_local` blocks web | partial | `trust/policy.rs:328`, refused at call time (`agent/tools.rs:1220`, browser `:1557`). Not blocked for `/research` turns (`:1216`), and the web tool schemas are still sent (removed only for Ollama with `local_web_only_research`, `agent/mod.rs:1158`) |
| f | `task` sub-agents share backend + model | **false** | Same backend, but `fast_target` picks the provider's newest `fast` model first and falls back to the parent's model (`agent/mod.rs:687-723`, used `:794`), effort `low` (`:752`). Not user-selectable |
| g | `estimateCost` never called | true (TS) | defined at `app/src/store/kernelIntents.ts:107`, no caller in `app/src`. The Rust `cost.estimate` behind it *is* used by the pre-turn budget check (`sdcp/methods.rs:1597`) |
| h | Two token counters disagree | true (three) | `context::tokens_of` = ASCII/4 + 1 per non-ASCII char (`context.rs:67-78`); `cost::tokens_in` = ASCII/4 + non-ASCII/2 (`trust/cost.rs:86-97`), history as `chars/4` (`:102`); UI `chars/4` (`LiveBar.tsx:41`, `LiveStats.tsx:37`, `FlowWork.tsx:502`, `stepKit.ts:88`). Bengali: `tokens_of` ≈ 2× `tokens_in`. Agent loop rescales with provider usage (`token_scale`, `agent/mod.rs:658`) |
| i | 22 tool schemas always sent | **false / partial** | 19 by default; 20 with `apply_patch` (GPT/Codex); 21–22 with the two vision tools; plus MCP tools (`agent/tools.rs:66-87, 170-380`, `agent/mod.rs:1164`). Filtering exists by capability (read-only sub-agent), research and Ollama-web, but **not by intent** |

Additional facts found (relevant to W5 caching): the system prompt contains `Today: {date}` (`agent/mod.rs:324`), so the
cached prefix changes every day; folding old tool output (`keep_small`, `agent/mod.rs:629`) rewrites earlier messages and
breaks the cache prefix.
