# Gates

A gate passes only when its section contains the line `Result: PASS`, written with the measured numbers by the last task of the phase.

## Gate 0

Audited by P0-17 on 2026-10-17, at commit `2020fc7`, version **3.0.0**.

| what the gate asks | what was measured |
|---|---|
| every test green on three systems | **Rust: 1,075 passed, 0 failed, 6 ignored** - 1,043 unit tests, and 32 across seven integration files (characterization 8, credential_guard 2, lifecycle 11, streaming 2, transaction_crash 3, vcr 6; `live_api` 2 ignored because it needs a live key). **App: 268 vitest tests**, 183 protocol methods agreeing in schema, `types.ts` and the daemon, 8 visual views at 0.000% pixel difference, a11y 0 violations; typecheck and eslint clean, Rust clippy `-D warnings` clean, and the dependency scan finds no advisory. Run on macOS locally **and** on GitHub's `macos-latest`; the same suite green on `ubuntu-latest` and `windows-latest` in [CI run 39011948634](https://github.com/skilleddesk/SD-Code/actions/runs/39011948634) - all six jobs, plus `desktop app`, `SDC Anywhere` and `Secret scan`. |
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

## Gate 2

## Gate 3

## Gate 4
