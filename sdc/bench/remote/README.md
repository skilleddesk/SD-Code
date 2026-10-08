# bench/remote

Benchmarks for SDC Anywhere (docs/remote/PERF.md). The scripts live next to the code they drive:

| What | Where | Run |
| --- | --- | --- |
| Latency of approvals, stream, reconnect; daemon memory | `web/e2e/perf.bench.test.ts` | `pnpm --filter @sdc/web bench:remote` |
| Push compose cost, email hand-off timer, sign-in link round trip (Phase 2) | `web/e2e/notify.bench.test.ts` | `pnpm --filter @sdc/web bench:remote` |
| Seal/open and handshake cost | `sdcd/src/anywhere/crypto.rs` (`bench` module) | `cargo test --release --lib bench_seal_and_open -- --ignored --nocapture` |

`results/` holds the raw JSON of each run, committed, so a regression is visible in a diff.
PERF.md says what each number is, and what topology produced it. Same-machine numbers are software overhead;
they are not what a phone on 4G will see.
