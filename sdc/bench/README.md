# SDC bench (v3 W1)

The measurements every v3 claim stands on. Reports are generated from `results/`; no number is typed by hand.

```text
fixtures/      four small projects the tasks run in (rust-ledger, ts-shop, py-notes, devops-site)
tasks/*.toml   the 40 plan tasks + 2 paired repeats (I4); tiers smoke (8) ⊂ medium (20) ⊂ full (40)
tasks/hidden/  tests copied in only after a run - the agent never sees them
tasks/reference/  a known-good solution per task (`sdcd bench validate` proves each task fails untouched
               and passes with its reference)
lang/          parallel corpus for the language-cost probe (A-14)
results/<label>/  meta.json, runs.jsonl, ledger.jsonl, report-bn.md
bin/           baseline binaries built from tags (not committed): bin/v1/sdcd = sdc-baseline-v1
```

## Commands

```sh
sdcd bench validate                                  # free: every task is meaningful and solvable
sdcd bench list --tier medium
sdcd bench run --models deepseek:deepseek-chat --tier smoke --dry-run      # plan and prices, no calls
sdcd bench run --models deepseek:deepseek-chat,anthropic-api:claude-sonnet-5-5 --tier smoke --max-usd 2
sdcd bench run ... --binary bench/bin/v1/sdcd --label baseline-v1          # measure the v1 binary
sdcd bench run ... --ablate <feature>                                        # one v3 feature off
sdcd bench competitors --models claude-code:claude-sonnet-5-5,codex:gpt-5.1-codex --tier smoke
sdcd bench lang --models deepseek:deepseek-chat                              # A-14, provider's own count
sdcd bench gen-tree --out /tmp/tree-50k && sdcd bench search --root /tmp/tree-50k --label <label>
sdcd bench report <label> [--baseline <label>]
sdcd bench compare <baseline> <candidate> --report-bn                        # the Q3 rule per model
```

## What a run is

Each task × model × run gets a fresh copy of its fixture (committed to a throwaway git repo) and one headless
agent turn through a daemon of the bench's own (scratch database, `SDC_BENCH_LEDGER` on, autonomy `auto`,
the task's step and time caps). Questions the agent asks are answered "proceed with your best judgement";
permission prompts are declined. The verifier then checks the answer (regex / rubric), copies the hidden
tests in and runs them, and checks that read-only tasks changed nothing.

The **ledger** records every model request beside the request (the request itself is byte-for-byte unchanged -
`the_ledger_matches_the_providers_usage_and_leaves_the_request_unchanged`): provider usage, and the input
split into system / tools / context / history / tool_results / user. The split is SDC's count apportioned to
the provider's total, so the parts always sum to exactly what the provider billed.

**Billable-equivalent** = uncached input + 0.1 × cached input + output × (output price / input price).
**USD** uses the catalogue price, cached input at a tenth (cache writes are not priced separately).
Statistics: median with a bootstrap 95% interval (fixed seed); McNemar's exact test for paired pass/fail;
Q3 = p ≥ 0.1, pass-rate drop ≤ 1 point, saving CI above 0.

## Notes

- Keys come from the OS keychain the app uses, so the bench binary must be built with `--features keychain`
  on Windows and macOS (`cargo build --release --features keychain`), or it will not see them.
- `--data-dir` moves the daemon's data folder (useful for a mock provider in `local-servers.json`); keys stored
  in the file fallback (Linux) live in the data folder and are then not found.
- `bench search` timings are with the OS file cache as it is; drop it first for a cold number and pass `--cold`.
- DevOps tasks use `grep` in their checks; the fleet tasks (I7, Docker 5/10/20) arrive with I7.

## Rules, tiers and your own tasks (P0-11)

A result is only worth believing with its noise beside it, so these are enforced in code (`sdcd/src/bench/rules.rs`):

- **A fixed seed** (`--seed N`, default 20261017) and, when stated, a temperature (`--temperature T`) are sent with every request and written into `meta.json`. (DeepSeek, OpenAI-compatible endpoints and Ollama accept both; a request with thinking on keeps the provider's own temperature.)
- **At least 3 runs per task** outside the smoke tier; `--allow-few-runs` accepts a result with no noise estimate and says so.
- **Paired runs** (same task, same run index, two builds) feed McNemar in `bench compare` (`stats::mcnemar_p`, tested against the binomial table).
- **Noise in every report**: how many tasks flipped between runs and how far apart the runs' pass rates were - in `report-bn.md` and in the machine-readable `summary.json` (`schema: 1`: label, tier, rules, per-model pass rate, noise, USD).
- **A budget**: `--max-usd`.

Tiers beyond smoke/medium/full: `hard` (what the baseline fails or flips - today `ml-bn`, plus your private tasks), `small` (the 8-9B local-model bench: long-document QA with a quoted answer, a multi-file feature, a rename across 24 modules, a long function that is easy to cut off, Bangla and Banglish instructions, a file near the context limit, an edit in a project with no tests) and `private`. `small` is not part of `full`, so every baseline's "the 40 tasks" keeps its meaning.

**Your own tasks** go in `tasks/private/` (git-ignored: never committed, never published) and may name a real project folder as their fixture; `sdcd bench private` prints where and shows an example. They join the `hard` tier automatically.
