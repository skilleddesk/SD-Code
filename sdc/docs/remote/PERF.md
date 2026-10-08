# SDC Anywhere - Performance

Status: Phase 1 and the software side of Phase 2 are measured, on one machine (section 5). Column "target" comes from plan section 7.2 (estimates). Measured columns stay empty until the matching phase is built and the benchmarks run, and until the thing being measured can be reached: **no row that depends on a deployed relay, a push service or a mail provider has a number yet** (rows 2 and 3 in particular: delivery time is Google's, Mozilla's, Apple's and Gmail's, not ours). Rows are filled with real p50/p95, never with the estimates.

## 1. Method

- **Harness:** `sdc/bench/remote/` (committed; DESIGN.md OQ-8 explains why not `_verify/remote/`). One script per scenario, JSON output, one run = 200 samples after 20 warm-up samples.
- **Reported:** p50, p95, max, sample count, failures. A run with failures is reported with the failure count, not hidden.
- **Network profiles** (applied on the browser side and checked with a ping probe):
  - `lan`: no throttle (sanity baseline)
  - `4g`: 40 ms RTT, 12 Mbps down / 4 Mbps up, 0% loss
  - `4g-lossy`: the `4g` profile plus 1% packet loss
  - `bd-4g` (when run from Dhaka): real mobile network, no emulation
- **Topology recorded with every run:** DO location hint, PC location and uplink, VPS location, browser/OS/device, build version, git commit.
- **Clocks:** each sample timestamps with one clock where possible; cross-machine intervals use round-trip echo, not wall-clock subtraction.
- **Cold vs warm:** every row is reported both with the Hub hibernated (first message after idle) and warm.
- **Repeat:** every release re-runs the suite and appends a dated section here.

## 2. Scenarios (plan 7.2) and where they become measurable

| # | Scenario | Target p95 (plan) | Phase | p50 | p95 | Date / commit |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Approval request reaches an open phone page | < 500 ms | 1 | 5.3 ms | 6.7 ms | 2026-10-08 (re-run after Phase 2), same machine, see section 5 |
| 2 | Web Push delivery | < 10 s | 2 | not measurable yet | | needs a real browser subscription and a deployed relay; the software share is 0.6 ms (section 5, Phase 2) |
| 3 | Email delivery to Gmail | < 90 s | 2 | not measurable yet | | needs the real provider (OQ-14) and a deployed relay; the relay's own timer is 37 ms late at p50 (section 5, Phase 2) |
| 4 | Tap Allow -> work resumes on PC | < 500 ms | 1 | 6.1 ms | 7.3 ms | 2026-10-08 (re-run after Phase 2), same machine, see section 5 |
| 5 | AI stream event visible in browser | < 500 ms | 1 | 2.4 ms | 2.8 ms | 2026-10-08 (re-run after Phase 2), same machine, see section 5 |
| 6 | Open folder, 200 items, local PC | < 700 ms | 2 | | | |
| 7 | Open folder, VPS | < 1.2 s | 2 | | | |
| 8 | Open a 100 KB file | < 1 s | 2 | | | |
| 9 | Content search, first result | < 2 s | 2 | | | |
| 10 | 10 MB download (throughput) | - | 3 | | | |
| 11 | 10 MB upload from phone (throughput) | - | 3 | | | |
| 12 | 100 MB upload with a forced disconnect (resume) | completes | 3 | | | |
| 13 | Terminal echo: predictive vs confirmed | confirmed 100-250 ms | 3 | | | |
| 14 | Preview first load | < 8 s | 4 | | | |
| 15 | Resume after network drop (`last_seq`) | < 5 s | 1 | 555 ms | 649 ms | 2026-10-08 (re-run after Phase 2), same machine, see section 5 |
| 16 | Per-message encrypt + decrypt cost | < 1 ms | 1 | 0.7 µs (256 B) · 2.2 µs (1 KB) · 33 µs (16 KB) · 0.51 ms (256 KB) | 0.8 µs · 2.3 µs · 49 µs · 0.57 ms | 2026-10-08 (re-run after Phase 2), release build, see section 5 |
| 17 | Kill reaches the engine while a download saturates `xfer` | < 500 ms | 3 | | | |

## 3. Extra measurements Phase 0 asked for

| Metric | Why | Phase |
| --- | --- | --- |
| Worker CPU ms per request, p95, for WebAuthn assertion verify, magic-link verify, push send | Free plan allows 10 ms CPU per invocation (DESIGN.md section 4) | 1 (push compose: proxy measured in Node, 0.8 ms p95; the rest needs the deployed Worker) |
| Messages per hour per scenario (browser-side and PC-side) | replaces the assumptions in the cost estimate | 1-3 |
| DO active duration (GB-s) per day for one account | validates hibernation | 1 |
| PC daemon extra RSS and CPU with remote enabled, idle and streaming | plan 7.3 says ~10-30 MB | 1 |
| Phone data used for a one-hour session | plan 7.3 says 2-10 MB | 2 |
| Added latency of the VPS path (PC <-> VPS exec-based file ops) and, if added, SFTP | decides DESIGN.md OQ-2 | 2-3 |

## 4. Regression rule

A release whose p95 for any row is more than 25% worse than the previous recorded run, or misses its target, is blocked until the row is explained in this file.

## 5. Results

### Phase 1 (0.17.0), 2026-10-08 (re-run the same day, after the Phase 2 changes to the Hub, the daemon and the page)

The numbers below are from that re-run (`bench/remote/results/phase1-2026-10-08.json` was rewritten by it); the first run, taken
before Phase 2, was 5.5 / 7.3 / 2.5 ms p50 for rows 1 / 4 / 5 and 554 ms for row 15. Nothing moved by more than the noise of
one machine (the regression rule in section 4 allows 25%), and adding push, email and the sign-in link to the Hub did not slow the
approval path: the new work happens in the alarm and in separate messages.

**Topology: the relay, the daemon and the browser code ran on one Windows 11 machine (loopback), software only.**
These numbers are the *overhead of the design*: sealing, the Durable Object hop (workerd via `wrangler dev`), opening,
and the daemon's checks. There is **no WAN in them**. A phone on 4G adds the radio's round trip on both legs
(phone to relay, relay to PC), roughly 2 x 40-120 ms by the plan's own assumption, so the plan's 500 ms targets
for rows 1, 4 and 5 are met by the software with a wide margin and are decided by the network. Rows that need the
deployed relay (real DO location, TURN, push) are not measured yet and stay empty.

Method: `web/e2e/perf.bench.test.ts` (`pnpm --filter @sdc/web bench:remote`), raw output in
`bench/remote/results/phase1-2026-10-08.json`. 100 samples per row after 10 warm-up, timestamps taken inside the
receiving callback (polling for the result inflated an earlier run by up to the poll interval and was removed).
Row 16 is `cargo test --release bench_seal_and_open -- --ignored --nocapture`, 2000 samples per size.

| Scenario | samples | p50 | p95 | max |
| --- | --- | --- | --- | --- |
| 1 request reaches an open page, measured from the start of the daemon's request | 100 | 5.3 ms | 6.7 ms | 7.9 ms |
| 1a for reference: the daemon's own `permission.request` call (one SQLite commit for the id) | 100 | 2.5 ms | 3.3 ms | 5.4 ms |
| 4 Allow: decision signed in the browser, sent, checked by the daemon, gate resolved, answer back (a round trip, so an upper bound on "work resumes") | 100 | 6.1 ms | 7.3 ms | 8.3 ms |
| 5 a stream event is shown after the daemon logs it | 100 | 2.4 ms | 2.8 ms | 3.8 ms |
| 15 socket lost to a live session until the page is back at the level it had, no passkey (the first retry waits 0.5 s +/- 25%) | 20 | 555 ms | 649 ms | 652 ms |
| 16 seal + open, 256 B / 1 KB / 16 KB / 256 KB | 2000 each | 0.7 / 2.2 / 33 / 506 µs | 0.8 / 2.3 / 49 / 567 µs | - |
| Full end-to-end handshake, both sides in one process | 200 | 0.70 ms | 0.92 ms | - |

Resources: the daemon's working set was 15.1 MB before SDC Anywhere was turned on and 28.5 MB with it on and one
browser connected: **+12.8 MB**, inside the plan's 10-30 MB estimate. CPU while idle was not sampled in this run.

Not measured here, and why: Worker CPU per request (workerd does not expose CPU time locally and freezes
`performance.now()`; it has to be read from the deployed Worker's analytics, DESIGN.md section 4); rows 2, 3, 6-14 and 17
belong to later phases.

A finding worth keeping: row 15 is dominated by the deliberate retry delay, not by the handshake (0.69 ms). The first
retry was 2 s in the first draft (2^n seconds); it is now 0.5 s doubling to 30 s, which is what the number above
measures.

### Phase 2 (push, email, sign-in link), 2026-10-08

**Topology: relay (wrangler dev on workerd, local D1), daemon, a mail catcher standing in for the provider, and the test code, all on one Windows 11 machine over loopback.**
**There is no push service, no mail provider and no deployed relay in these numbers, so rows 2 and 3 of section 2 still have no delivery
figure, and cannot get one until a real browser is subscribed and a real provider is configured (OQ-14).** What is measured is the
software's own share of each path.

Method: `web/e2e/notify.bench.test.ts` (`pnpm --filter @sdc/web bench:remote`), raw output in
`bench/remote/results/phase2-2026-10-08.json`.

| What | samples | p50 | p95 | max |
| --- | --- | --- | --- | --- |
| Push, compose: RFC 8291 encrypt (ECDH, two HKDF, AES-GCM) of the 45-byte message | 200 | 0.44 ms | 0.57 ms | 1.3 ms |
| Push, compose: VAPID JWT signature (ECDSA P-256) | 200 | 0.15 ms | 0.20 ms | 1.0 ms |
| Push, compose: the whole `send()` to a stub push service | 200 | 0.61 ms | 0.80 ms | 1.5 ms |
| Email hand-off: how late the relay's alarm is. Delay set to 1 s; time from the daemon's request to the provider call, minus 1 s | 8 | 37 ms | 52 ms | 52 ms |
| Sign-in link: request accepted -> mail handed to the provider (D1 writes, then the Worker's deferred send) | 5 | 26 ms | 40 ms | 40 ms |
| Sign-in link: button -> pairing offer back in the browser (Worker, D1 `UPDATE`, Durable Object, daemon, and back) | 5 | 39 ms | 51 ms | 51 ms |

How to read them:

- **Push compose is Node, not workerd.** workerd freezes its clock during CPU-only work, so Worker CPU cannot be read locally; the same WebCrypto
  calls were timed in Node 24 as a proxy. At 0.8 ms p95 the cost is far inside the Free plan's 10 ms per invocation, but the real figure is
  the deployed Worker's analytics, and the sign-in link's own CPU (one hash, one D1 `UPDATE`) was not measured at all.
- **The email hand-off figure is the timer, not the mail.** With the 60 s default the relay's alarm fires about 40 ms late on this machine; what a
  provider and Gmail add after that (plan estimate 5-60 s) is not known.
- **5 and 8 samples are small.** The sign-in numbers have 5 because the daemon makes at most one pairing offer per 10 s and the relay allows 5 links per
  computer per hour, and the email figure has 8 because the relay sends at most 10 approval emails per computer per hour (both are limits
  of the design, not of the benchmark). Treat the p95 column of those rows as "about the max".

Not measured, and why: Worker CPU for the sign-in link and for WebAuthn (needs the deployed Worker); real push delivery on Android (FCM), Firefox
(autopush) and iOS (APNs, only for a page on the Home Screen); real email delivery to Gmail and its SPF/DKIM/DMARC result (DNS-RECORDS.md section 6
has the checklist for the owner to run once the provider is confirmed).
