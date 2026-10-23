# Gates

A gate passes only when its section contains the line `**Released:** [`v3.0.0`](https://github.com/skilleddesk/SD-Code/releases/tag/v3.0.0) - macOS `.dmg` (Apple Silicon and Intel), Windows `.exe` and `.msi`, Linux `.AppImage`, `.deb` and `.rpm`, with `SHA256SUMS.txt`; the `release` workflow built every one of them green.

Result: PASS`, written with the measured numbers by the last task of the phase.

## Gate 0

Audited by P0-17 on 2026-10-17, at commit `2020fc7`, version **3.0.0**.

| what the gate asks | what was measured |
|---|---|
| every test green on three systems | **Rust: 1,075 passed, 0 failed, 6 ignored** - 1,043 unit tests, and 32 across seven integration files (characterization 8, credential_guard 2, lifecycle 11, streaming 2, transaction_crash 3, vcr 6; `live_api` 2 ignored because it needs a live key). **App: 268 vitest tests**, 183 protocol methods agreeing in schema, `types.ts` and the daemon, 8 visual views at 0.000% pixel difference, a11y 0 violations; typecheck and eslint clean, Rust clippy `-D warnings` clean, and the dependency scan finds no advisory. Run on macOS locally **and** on GitHub's `macos-latest`; the same suite green on `ubuntu-latest` and `windows-latest` in [CI run 39013102624](https://github.com/skilleddesk/SD-Code/actions/runs/39013102624) on `main` at `a9c590f`, the commit tagged `v3.0.0` - all six jobs, plus `desktop app`, `SDC Anywhere` and `Secret scan`. |
| no model call it was not allowed to make (Manual Pin) | the full-tier bench ran with the model pinned to `deepseek:deepseek-chat` while `router.fast`, `v3.failoverOrder` and `cascade.strong` all named other models: **40/40 tasks passed**; of its 313 ledger rows **241 are model calls, every one on the pinned model, 0 on any other** (the other 72 rows are `main-reads` file reads with no model call); $0.0646 spent, spread 0.0, 0 flaky tasks (`sdc/bench/results/gate0-pin`, reported in Bengali there). |
| no silent truncation (overflow test) | 7 `guard::context` tests: a prompt that does not fit is never sent (it is split into smaller steps or refused with a reason), the token estimate never undercounts a real tokenizer, the answers reserve counts against the window, and a sentinel at the start of the conversation is never in a prompt that could be cut. |
| no plaintext key | `sdcd keys stray` finds 10 lines in the tree and every one is a fixture or a placeholder: `sk-ant-bogus…`, `hunter2-not-real`, `postgres://shop:shop@db`, the RFC 6238 test secret, and two `sk-…` constants inside tests. **0 real keys in tracked files**; the two live scripts under `_verify/` hold a local password but are gitignored (`.gitignore:146`) and never shipped. |
| baseline, speed baseline, Step Trace and both benches ready | ground truth: **119/120 = 99.2%**, $0.202, three runs a task, 0 flaky (`sdc/bench/results/baseline-2026-10-17-deepseek`); the speed and Step Trace baseline frozen at `sdc/bench/baselines/2026-10-17` (`ledger.jsonl`, 237 KB, every step's model, tokens, wall time and cost); the full-tier bench and the Manual Pin bench both run end to end from the CLI. |

**Found and fixed while auditing this gate** (each with a test):

- **A shadow repository per command, not per operation** (`git::checkpoint` / `git::restore`). Each git command resolved the shadow repository again from the data directory; when the data directory moved between two commands - it comes from the environment - `add -A` staged in one repository and `commit --allow-empty` committed in another, a *fresh* repository commits an **empty** tree, and restoring that emptied the project. It showed up as the rewind test coming back with no files in a folder, roughly one run in six. An operation now opens one `Shadow` and runs every command through it.
- **The data directory under `cargo test`** is now one folder per test process, never removed: a test that pointed `SDC_DATA_DIR` elsewhere and a folder deleted while another thread wrote inside it were the other half of the same fault.
- **The Linux sandbox row**: the Verify test keyed on the filesystem guarantee alone and called every Linux runner (where only `unshare` exists, network isolation alone) a failure.
- **The characterization golden** recorded this machine's settings (`dashscope-us`) where CI produced the default (`dashscope-intl`); the test now isolates its data directory.
- **`zip` 2.4.2** carries RUSTSEC-2026-0367 and -0368: 9.0.4, three calls, no API change.
- **Four tests that only Windows failed**: a `;` that `cmd` does not split, two loops of output written in `sh`, and a Seatbelt profile test that asserted macOS path canonicalization.

Result: PASS

## Gate 1

**Status: not run.** What changed in 3.1 is that it can now be run honestly. The statistics plan (written 2026-10-18, before any Gate 1 number exists) is below.

### Why the plan had to change

Gate 1's rule is "same 9B model, Local-only, the Small Model Engine against plain prompting, paired runs, McNemar p < 0.05". With the 8 small-model tasks that existed in 3.0.3 this is **close to unachievable**: simulated power (below) is 3-16 % even for a 20-point improvement, so a real improvement would almost always be reported as "no evidence". A gate that cannot pass is not a gate. The small tier is now **40 tasks** (the original 8 + 32 on three new realistic projects: `py-inventory`, `py-textkit`, `py-billing`; feature, bugfix, rename, no-tests, question-answering; English, Bangla, Banglish, Arabic, German), every one proven by `sdcd bench validate` to fail on the untouched project and pass with its reference solution.

### The method

* **Unit of analysis: the task, not the run.** Runs of one task are correlated (a hard task is hard every time), so counting every run as an independent pair overstates the evidence. For each task, count passes of the SME arm and of the plain arm over the same number of runs (>= 5; the bench refuses fewer than 3), and call the task *better*, *worse* or *tied*.
* **Primary test:** exact two-sided sign test on the better/worse counts (ties dropped), alpha 0.05, SME must also be the one with more better tasks. This is McNemar's exact test applied at task level.
* **Secondary, reported but not decisive:** McNemar on all (task, run) pairs (`sdcd bench compare`, as before). It ignores the correlation above, so it is optimistic and is labelled so.
* **Same everything else:** same model digest (`identity` from the probe), fixed seed, temperature written in `meta.json`, same fixtures, hidden tests copied in after the run, Local-only mode, `--ablate` for the plain arm.
* **Hard zeros, no statistics:** cut-off answers, invented quotes and silent mis-patches must be 0 in the SME arm.
* **Speed:** the Speed Gate (median within 5 % of baseline).

### Power (simulated, 4,000 trials per cell, baseline pass rate 40 %, alpha 0.05)

| tasks x runs | +10 points | +15 points | +20 points |
|---|---|---|---|
| 8 x 3 (3.0.3) | 3 % | 5 % | 10 % |
| 8 x 5 | 5 % | 9 % | 16 % |
| 20 x 3 | 12 % | 24 % | 41 % |
| 20 x 5 | 19 % | 39 % | 61 % |
| 40 x 3 | 26 % | 53 % | 77 % |
| **40 x 5 (the plan)** | **41 %** | **73 %** | **92 %** |

So the gate can detect a real **+20-point** improvement 92 % of the time, a +15 one 73 % of the time, and will usually miss +10. If the true effect is smaller than that, the honest result is "not shown", and the report must say so rather than add runs until a p-value appears (the number of runs is fixed in advance: 5).

### Cost and where it runs

40 tasks x 5 runs x 2 arms = 400 turns on the local 9B model: no API cost, but hours of GPU time. **This is OWNER-TO-RUN on the Windows PC (RTX 4060 8 GB)**; this Mac has no model and does not run it:

```
# pilot (one run, the plain arm) - look at the baseline first
sdcd bench run --models ollama:qwen3.5:9b --tier small --runs 1 --label gate1-pilot-plain --ablate-group sme
# the real test
sdcd bench run --models ollama:qwen3.5:9b --tier small --runs 5 --label gate1-sme
sdcd bench run --models ollama:qwen3.5:9b --tier small --runs 5 --label gate1-plain --ablate-group sme
sdcd bench compare gate1-plain gate1-sme --report-bn
```

**If the pilot's plain arm passes more than 85 % of the tasks, run the `hard` tier instead** (`--tier hard`, 8 or more tasks): a baseline that already passes almost everything leaves no room to show an improvement, and a gate that cannot show one is not a gate.

**What was wrong with the earlier command (I-1).** It ablated `sme.budget,sme.step_profile,sme.prompt_diet` only. `sme.step_profile` is off by default anyway, so the plain arm differed from the SME arm by two switches; retrieval, tool-call rescue, constrained retries, quote verification, confidence, memory, the spec brief, the decomposer, the fast path and the Code Graph stayed on in **both** arms. The comparison was "SME" against "SME without its budget", and its answer would have been read as "SME against nothing". `--ablate-group sme` turns off every `sme.*` switch, `runtime.decompose`, `runtime.fastpath` and `graph.v1` together (the list is `bench::group_features`, and a test checks it), and leaves `edit.transaction`, `exec.sandbox` and `local.runtime_guard` on in both arms - they guard the run, and a plain arm without them would be unsafe, not plain.

Status: **owner-later** (it needs the local model; this build does not run Gate 1).

Your own tasks go in `bench/tasks/private/` (git-ignored; `sdcd bench private` prints the folder and an example) and join the `hard` tier.

## Gate 2

## Gate 3

## Gate 4
