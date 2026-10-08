# SDC Anywhere - Design (Phase 0)

Status: **Phase 0, awaiting "Phase 0 approved".** No feature code exists. Master plan: [`../SDC-ANYWHERE-PLAN-v2.md`](../SDC-ANYWHERE-PLAN-v2.md) (Bengali; replaces any older plan - no `SDC-ANYWHERE-PLAN.md` exists in the tree).
Companion files: [`THREAT-MODEL.md`](THREAT-MODEL.md), [`DNS-RECORDS.md`](DNS-RECORDS.md), [`PERF.md`](PERF.md).
Code surveyed at `sdcd` 0.16.1 (`Cargo.toml`), 2026-10-08.

The nine rules of plan section 1 are treated as invariants. Where this document finds that the code makes a rule hard to keep, it says so under [Open questions](#open-questions) instead of weakening the rule.

---

## 1. What is being built, in one paragraph

A browser (PWA at `sdc.skilleddesk.com`) controls the user's own PC-side `sdcd`. The PC only dials **out** (one WSS to a per-account Durable Object "Hub"). Everything between browser and daemon is end-to-end encrypted, so Cloudflare routes ciphertext. The daemon reaches VPSs itself over the SSH connection it already holds; the browser never touches a VPS. Every state-changing remote request goes through the existing Trust Kernel (policy, checkpoint, audit).

## 2. Repo reality vs. the plan's paths

| Plan says | Repo has | Decision |
| --- | --- | --- |
| repo root `sdc/` | git root is `H:\SDC`; the project is `H:\SDC\sdc\` (`app/`, `sdcd/`, `protocol/`, `docs/`, `design/`); `.github/` and the authoritative `.gitignore` are at `H:\SDC` | all plan paths are relative to `H:\SDC\sdc\` except `.github/`, root `.gitignore`, `CHANGELOG.md`, `SECURITY.md` |
| `docs/remote/SDC-ANYWHERE-PLAN-v2.md` | file is at `sdc/docs/SDC-ANYWHERE-PLAN-v2.md` | left in place; new docs are in `sdc/docs/remote/`. Move it only if you want (links in this folder assume the current place) |
| `protocol/schema/remote/` | `protocol/` holds `sdcp.schema.json`, `types.ts`, `check.mjs`, `models.json`; no `schema/` dir; `protocol` is commented out of `pnpm-workspace.yaml` | see OQ-6 |
| `_verify/remote/` | `_verify/` exists twice (`H:\SDC\_verify`, `H:\SDC\sdc\_verify`) and is **gitignored** (it holds browser profiles) | see OQ-8 |
| `sdcd/src/trust/` "hook: permission broker" | `trust/` has `cost, kill, ledger, policy, proof, scan, score`. The permission broker is `agent/gate.rs` | the hook goes in `agent/gate.rs` (section 3, row `approval_router`) |
| `sdcd/src/remote/` | `auth/remote.rs` + `docs/REMOTE.md` already mean "remote **hosts** (VPS over SSH)" | see OQ-11 (naming) |
| `cloud/`, `web/` | do not exist | new; separate pnpm packages (add to `pnpm-workspace.yaml` in Phase 1) |

## 3. Module map (plan section 12 -> real code)

Rule: **reuse, do not duplicate.** "Reuse" means the new module calls the existing function; it does not copy it.

| Plan module | Maps to / reuses | New code |
| --- | --- | --- |
| `remote/mod.rs`, `identity.rs`, `pairing.rs`, `device_registry.rs` | `store/` (SQLite; add tables via the existing migration path in `store/sqlite.rs`), `auth/keychain.rs` for the device private key | new module; device table; pairing/SAS |
| `crypto.rs` (HPKE, AEAD, sig verify, nonce store) | `sha2`, `hex`, `base64` already in `Cargo.toml` | **new crates needed** (HPKE, AEAD, P-256/Ed25519, BLAKE3) - OQ-5 |
| `session.rs` (capability levels, operate window, idle lock) | `trust/policy.rs` supplies *limits* only (OQ-4); levels are new | new |
| `mux.rs` (channels, priority, resume `last_seq`) | `sdcp/events.rs` `EventLog::since(seq)` is already "give me everything after seq N" and is the resume primitive; `sdcp/notifications.rs` `Fanout` is the live fan-out | priority/flow-control is new |
| `relay_client.rs` (outbound WSS) | `tungstenite` is present but **without TLS** (`features=["handshake"]`); `ureq` has `tls`. `main.rs` already runs tokio listeners | WSS client + TLS dep - OQ-5 |
| `envelope.rs` (`ActionEnvelope`, `action_hash`) | `sdcp/envelope.rs` (`Envelope`, `Response`, `ErrorObject`) is the SDCP request frame - **different thing**; hash via `sha2` | `ActionEnvelope` is new; canonical JSON is new (`serde_json` has no canonical mode) |
| `approval_router.rs` | `agent/gate.rs`: `ask()` blocks on an mpsc channel keyed `perm-{turn}-{call}`, `resolve(id, decision)` answers it; `sdcp/methods.rs` `permission.request/resolve`; `Fanout` pushes `EngineEvent::Permission` to listeners | router subscribes to the same event and calls `gate::resolve`. Needs richer payload and more decisions - OQ-3 |
| `blast_radius.rs` | `trust/policy.rs` (`protected`, `always_asks`, `denies`, `max_files_per_turn`), `trust/scan.rs`, `agent/gate.rs::looks_dangerous`, `pty::denied_reason_line` | tree-sitter parse is new and optional - OQ-9 |
| `fs_gateway.rs` | local: `fs/mod.rs` (`guard`, `real_path`, `strictly_inside`, `list`, `read_capped`, `search`, `find_names`, `hash_file`, `write`, `rename`, `remove`, `mkdir`). VPS: `ssh/ops.rs` (`list`, `read`, `write`, `rename`, `remove`, `mkdir`, `stat`, `search`, `find_names`, `git_status`, `git_diff`, `link_guard`, `guard`). SDCP already has `fs.*` with `hostId` | roots/`path_id` indirection, pagination cursors, protected-path rules, ripgrep streaming. **Not SFTP** - OQ-2 |
| `xfer.rs` | `fs::hash_file`, `fs::read_capped`; VPS bytes via `ssh/ops.rs` read/write; `ssh/native.rs` holds the one russh connection per host | chunking, resume, BLAKE3, quarantine inbox, PC<->VPS copy |
| `pty_gateway.rs` | `pty/mod.rs` (`PtyManager::open/write/output_since/close`, `denied_reason_line`, `line_command`, `shell_for_line`), `ssh/bridge.rs` (a process that behaves like `ssh` for the Terminal), `ssh/ops.rs::shell` | guarded line mode, raw mode gating, asciicast recording |
| `preview.rs` | `ssh/native.rs::forward` (port forward for the VPS) and `sdcd/src/preview.rs` (loopback proxy; **name differs, purpose differs**: it strips frame-blocking headers for the desktop frame) | HTTP-over-E2E request forwarder, localhost-only, port allow-list |
| `remote_input.rs` | `intent/` (Intent Engine), `sdcp/intent_methods.rs`, `sdcp/agent_methods.rs`, `session.open`, `agent/` | thin adapter; no new agent logic |
| `os/{idle,sleep}_*.rs` | nothing (`host/` is doctor checks and env path) | new, per-OS |
| checkpoint before write | `checkpoints::create/list/turn_of`, `rewind/`, `ssh/ops.rs::shadow_checkpoint/shadow_restore` | none - call them |
| audit | `trust/ledger.rs`: fed from `EventLog::append`, so any event appended is audited; `audit.verify` exists | new event types for remote actions (device id, capability level, `action_hash`) |
| Proof Pack with remote decisions | `trust/proof.rs` | add fields |
| kill | `trust/kill.rs` `cancel_all`, `agent` `engines::cancel` | wire `control.kill` to it |
| host list/status/2FA | SDCP `host.*`; `ssh/hostkey.rs`; `ssh/totp.rs` | "phone supplies a code" prompt path is new |
| cost display on cards | `trust/cost.rs` `estimate` | none |
| `cloud/` Worker + Hub DO + D1 + email adapter | none | new TypeScript package |
| `web/` PWA | `app/` (React + Vite + Tailwind; design tokens, components, 10 languages) | share tokens/components - extract to a shared package or import by path; decide at Phase 1 |
| desktop Settings -> SDC Anywhere | `app/src/…` settings pages, Tauri | new tab |

### 3.1 Remote requests reuse SDCP methods

The plan defines parallel messages (`fs.list{host,path_id,cursor}`, `pty.open{host,mode}`...). SDCP already has `fs.list/read/write/stat/search/rename/delete/mkdir`, `pty.open/write/resize/close/output`, `shell.run`, `permission.request/resolve`, `checkpoint.*`, `rewind.*`, `host.*`, with `hostId` on the envelope. **Proposal:** the remote mux tunnels ordinary SDCP envelopes through the capability gate, and adds only what SDCP lacks (pairing, capability unlock, `fs.list` pagination cursor, `xfer.*`, `preview.*`, `approval.*`). This keeps one definition of each operation, and `protocol/README.md` forbids inventing SDCP shapes outside the schema. See OQ-6.

### 3.2 Data flow for one approval (example 1)

1. Engine calls `gate::ask` -> `EngineEvent::Permission` -> event log (audited) -> `Fanout`.
2. `approval_router` (subscribed) builds an `ActionEnvelope`, computes `action_hash`, seals it for each trusted device, hands it to the mux.
3. Hub delivers ciphertext live; if no socket, Web Push (no content) -> after `escalate_to_email_after_sec`, email (no content).
4. Phone shows the card, user passkey-signs `action_hash`. Reply: sealed decision + signature.
5. `approval_router` verifies signature, nonce, expiry, device not revoked, hash equals the pending envelope, then `gate::resolve(permission_id, "allow_once")`.
6. Ledger records who approved, from which device.

## 4. Cloudflare plan and cost

Verified 2026-10-08 against Cloudflare's current docs (Durable Objects, Workers, D1, Queues, Pages pricing pages). Re-check before purchase; prices change.

| Product | Free plan | Workers Paid ($5/month minimum) | Needed here |
| --- | --- | --- | --- |
| Durable Objects | **SQLite-backed only**; 100,000 requests/day; 13,000 GB-s/day; 5M row reads/day, 100k row writes/day | 1M requests/month then $0.15/M; 400,000 GB-s/month then $12.50/M GB-s; SQLite 25B reads / 50M writes included; storage 5 GB-month then $0.20/GB-month | Hub, with **WebSocket Hibernation** (available on both plans; incoming WS messages bill at 20:1, outgoing and pings free; hibernated objects pay duration only while processing) |
| Workers | 100,000 requests/day; 10 ms CPU/invocation | 10M requests/month then $0.30/M; 30M CPU-ms/month then $0.02/M | `/api`, WebAuthn verify, magic link, push send |
| D1 | 5M reads/day; 100k writes/day; 5 GB | 25B reads, 50M writes included; storage 5 GB then $0.75/GB-month | account, device public keys, push subscriptions |
| Queues | **not available** | 1M operations/month then $0.40/M; 3 operations per message (write, read, delete) per 64 KB | email send with retry (optional, see below) |
| Pages / static assets | static asset requests free and unlimited on both; Functions share the Workers allowance | same | PWA files |
| Web Push | no Cloudflare product. VAPID JWT (ES256) + RFC 8291 `aes128gcm` encryption with WebCrypto inside a Worker, POST to the browser vendor's push endpoint; subrequests are not billed | same | push notifications |
| TURN (Phase 4) | not priced in this pass | - | **unverified**; price it at Phase 4 |

### Which plan?

* **Phase 1 prototype:** Free is technically enough if the email escalation uses a Durable Object alarm instead of Queues.
* **Recommended from Phase 1 on: Workers Paid ($5/month).** Reasons: Queues (retryable email) needs it; Free gives 10 ms CPU per invocation, and WebAuthn assertion verification plus JSON/crypto per request has not been measured against that limit (PERF.md will measure it, p95 CPU); Free has daily hard caps that would silently stop approvals mid-day.
* The Free plan is a fallback, not the design target.

### Estimate: one active account (assumptions are mine)

Assumptions: PC heartbeat every 30 s all day (2,880 messages/day), 8 active hours/day with ~20,000 incoming browser/PC messages (approvals, listings, chat, terminal keystrokes), ~50 D1 writes/day, ~5% of the day non-hibernated at 128 MB, 20 notification emails/day.

| Item | Per month | Included | Cost |
| --- | --- | --- | --- |
| DO requests | (2,880 + 20,000) / 20 x 30 = ~34,000 | 1,000,000 | $0 |
| Worker requests (API, login, push) | ~30,000 | 10,000,000 | $0 |
| DO duration | 0.125 GB x 129,600 s = ~16,200 GB-s | 400,000 | $0 |
| D1 / DO SQLite writes | ~3,000 | 50,000,000 | $0 |
| Queues | 20 x 30 x 3 = 1,800 ops | 1,000,000 | $0 |
| **Workers Paid base** | | | **$5.00** |
| Email provider | 600 mails (SendKnot publishes $0.60 per 1,000 transactional) | - | ~$0.36 |
| **Total** | | | **about $5.4 / month** |

Headroom is large (the account uses roughly 3 percent of the DO request allowance), so the cost is the $5 base until there are many accounts. The estimate is from stated assumptions, not measurement; PERF.md will replace it with measured message counts per scenario. Large transfers are the cost risk (relay messages), which is why plan section 7 uses path-only attach and Phase 4 P2P.

No `wrangler` command that creates or deploys resources was run in Phase 0.

## 5. Phase map (unchanged from the plan, with repo-specific notes)

| Phase | Version | Notes from the code survey |
| --- | --- | --- |
| 1 | 0.17 | needs the new crates (OQ-5), the `gate.rs` payload extension (OQ-3), machine-level settings storage (OQ-4), and a decision on keychain-default (OQ-7) before identity keys are stored |
| 2 | 0.18 | `ssh/ops.rs` already returns lists/reads for VPS; pagination and `path_id` are the main new work |
| 3 | 0.19 | resumable 100 MB upload: no SFTP in tree (OQ-2) |
| 4 | 0.20 | preview HMR, WebRTC, Two-Person |
| 5 | 0.21+ | post-quantum hybrid |

CI: only `release.yml` and `secret-scan.yml` exist. Plan section 14 asks for Windows/macOS/Linux test CI and Playwright; that workflow is new work in Phase 1.

---

## Open questions

Each item is a conflict between the plan and the code (or inside the plan). A recommendation is given; none is acted on until you answer.

**OQ-1 Where the plan lives.** Plan says `docs/remote/SDC-ANYWHERE-PLAN-v2.md`; it is at `sdc/docs/`. *Recommend:* leave, or move it with `git mv`; your call.

**OQ-2 No SFTP in the codebase.** The plan says VPS access is "SFTP" (`fs_gateway`, `xfer`, PC<->VPS copy). `Cargo.toml` has `russh` with `default-features = false` and no SFTP crate, and `grep -i sftp` over `sdcd/` finds nothing. VPS file operations in `ssh/ops.rs` run shell commands over exec channels. *Options:* (a) add `russh-sftp` (new dependency, resumable offsets native); (b) keep exec-based `dd`/`cat`/`tail -c +N` with BLAKE3 checked on the PC (no new dep, slower, depends on remote tools, which `ssh/ops.rs` already assumes). *Recommend (b) for Phase 2-3, measure, then decide (a) from PERF.md numbers.*

**OQ-3 Approval payload and decisions.** `gate::ask` carries `title, sub, action, target, risk, explain`; the plan's `ActionEnvelope` also needs `host, cwd, args, file_hashes, blast_radius, rollback, est_cost_usd, expires_at`. Decisions today: `allow_once|allow`, `always_allow`, `show_me`, deny; the plan adds scoped-allow (30 min), edit-then-allow, deny-with-reason, deny-and-pause. `always_allow` currently means something broader than "30 min, pattern + cwd + host". *Recommend:* keep the local desktop decisions unchanged; add a separate remote decision enum and map it to `Decision` at the router; never reuse `always_allow` for remote.

**OQ-4 Settings: per project or per machine.** Plan section 9 puts `[remote]` in `.sdc/policy.toml`. That file is per project, `PolicyFile` uses `deny_unknown_fields` (a new table breaks parsing today), and `.sdc/policy.toml` is already in `DEFAULT_PROTECTED`. But `remote.enabled`, device list, idle lock, quiet hours are machine-level, and `remote_forbidden` must not be removable by a project file. *Recommend:* machine-level remote settings live in the daemon store (editable only from the desktop app); project `policy.toml` may only **narrow** (`remote.fs.roots`, protected paths, preview ports, `allow_raw_terminal=false`) via a new optional table added to `PolicyFile`; a hard-coded minimum for `remote_forbidden` in code.

**OQ-5 New dependencies.** Not in `Cargo.toml`: HPKE/AEAD, ECDSA P-256 (WebAuthn ES256) or Ed25519, BLAKE3, zstd, a TLS-capable WebSocket client, a canonical-JSON helper, and (optional) tree-sitter. The release profile is `opt-level = "s"`, and the project values a small, auditable binary. *Recommend:* approve a short, named list at the start of Phase 1 (RustCrypto family + `blake3` + `tokio-tungstenite` with rustls) and record each in `SECURITY.md`; defer zstd and tree-sitter.

**OQ-6 Protocol rules.** `protocol/README.md`: schema-first, nothing may invent SDCP shapes; the planned `schema/` layout is not created yet. The plan's `fs.list{host,path_id,cursor}` shares a name with the existing SDCP `fs.list`. *Recommend:* tunnel existing SDCP methods (section 3.1); add new methods to `protocol/sdcp.schema.json` first; put remote-only frames in `protocol/remote.schema.json`; enable `protocol` in the pnpm workspace when the TS types for them exist.

**OQ-7 Where the device key lives.** `keychain` is an optional Cargo feature, `default = []`, and on Linux the documented behaviour is a file fallback. SDC Anywhere's daemon identity key and pairing secrets are exactly what principle 7 protects. *Recommend:* remote refuses to enable unless the OS keychain backend is active (or the user explicitly accepts a file-protected key, shown in Settings and the Proof Pack). Windows (DPAPI) and macOS (Keychain) are fine; Linux needs a decision.

**OQ-8 Benchmarks vs `.gitignore`.** `_verify/` is gitignored everywhere (it contains Edge profiles). A benchmark folder `_verify/remote/` would never be committed or run in CI. *Recommend:* put the harness in `sdc/bench/remote/` (committed), or un-ignore only `_verify/remote/` with `_verify/*` + `!_verify/remote/`. PERF.md assumes `sdc/bench/remote/` until you decide.

**OQ-9 Terminal classifier.** Plan: `tree-sitter-bash` / PowerShell parsing. Today: `looks_dangerous` (word matching), `policy.always_asks` (substring), `pty::denied_reason_line`. *Recommend:* Phase 3 starts with the existing classifier plus a "fail closed on parse doubt" rule (anything with `$(`, backticks, `eval`, here-docs, `|` to a shell is Critical); add tree-sitter only if tests show false negatives.

**OQ-10 Protected files vs the file guard.** Plan 5.3: protected paths (`.env`, keys) may be *opened* at Critical level. `fs::guard`/`blocked_reason` today **refuses** `.env*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa`, `id_ed25519`, `.npmrc`, `.netrc`, `credentials` outright ("not warned about, refused"). *Recommend:* keep that hard block for the remote path, and let Critical level open only policy-protected paths that are not hard-blocked (`wp-config.php`, `*.sql`, `backup/**`). `.env` is never remotely readable. Tell me if you want otherwise.

**OQ-11 Naming.** `remote` already means SSH VPS hosts (`auth/remote.rs`, `docs/REMOTE.md`). Reusing it for the browser feature will confuse code, docs and the `[remote]` policy table. *Recommend:* module `sdcd/src/anywhere/`, docs folder stays `docs/remote/` as you named it, settings table `[anywhere]`.

**OQ-12 Fixed minimum for `remote_forbidden`.** In the plan it is a configurable list. Anyone able to edit it could empty it. Principle 6 should be code, not config (see OQ-4).

**OQ-13 Account model.** Plan leaves "per user vs team". *Recommend:* one account per user, one Hub per account, teams in Phase 4+ (matches the plan's own suggestion).

**OQ-14 Email provider unconfirmed.** `H:\emailapi.txt` holds a single bare 58-character value with no key name and no recognisable vendor prefix; it cannot be attributed from its shape alone. Your existing root records (`mx.sendknot.com`, DKIM selector `sk-cef511db`) point to SendKnot, so DNS-RECORDS.md treats SendKnot as the likely provider but writes both branches. *Needed from you:* confirm that the key was created in the SendKnot console and that `skilleddesk.com` shows as verified there.

**OQ-15 Existing `sdc` A record.** `sdc A 109.199.108.216` must be deleted by you right before the Custom Domain is added (DNS-RECORDS.md section 4). *Needed from you:* confirm nothing is served from that VPS at `sdc.skilleddesk.com`, and whether `skilleddesk.com` sends an HSTS header with `includeSubDomains` (check list in DNS-RECORDS.md).

---

## Decisions (owner accepted all recommendations, 2026-10-08)

The owner said to follow every recommendation above. They apply from Phase 1, **after** the literal "Phase 0 approved".

| OQ | Decision |
| --- | --- |
| 1 | plan file stays in `sdc/docs/` |
| 2 | exec-based VPS file ops for Phases 2-3; revisit `russh-sftp` from PERF.md numbers |
| 3 | separate remote decision enum mapped to `Decision` at the router; `always_allow` never used remotely |
| 4 | machine-level settings in the daemon store; project policy may only narrow; `remote_forbidden` base set is a code constant |
| 5 | RustCrypto family + `blake3` + `tokio-tungstenite` (rustls) approved for Phase 1; each recorded in `SECURITY.md`; zstd and tree-sitter deferred |
| 6 | tunnel existing SDCP methods; new methods go into the schema first; remote-only frames in `protocol/remote.schema.json` |
| 7 | remote refuses to enable without an OS keychain backend unless the user accepts a file-protected key, shown in Settings |
| 8 | benchmarks in `sdc/bench/remote/` |
| 9 | existing classifier + fail-closed rule first |
| 10 | `.env` and other hard-blocked files are never remotely readable |
| 11 | module `sdcd/src/anywhere/`, settings table `[anywhere]` |
| 12 | see 4 |
| 13 | one account per user, one Hub per account |
| 14, 15 | **still need facts from the owner** (email key provider; what the old `sdc` A record serves) |

---

## Phase 1 as built (0.17.0, 2026-10-08)

Where the build differs from the plan, and why:

| Plan | Built | Reason |
| --- | --- | --- |
| "passkey login" to the relay | the relay authenticates a **device key** (ECDSA, non-extractable) by challenge-response; the passkey is verified by the **daemon** for unlocks, approvals and pairing | trust starts at the daemon, not at a server (plan principle 5 and threat T28); there is no server-side session to steal |
| D1 holds accounts, devices, push endpoints | Phase 1 keeps device public keys in each Hub (SQLite in the Durable Object), re-taught by the daemon on every connect; `migrations/0001_init.sql` creates the D1 tables Phase 2 uses (push endpoints, magic links) | nothing in Phase 1 needed D1 |
| mailbox with 24 h TTL | none: the daemon is the only store of open requests, and a returning browser is shown them on unlock | less data at rest on the relay; the push/e-mail nudge carries no content anyway |
| `mux.rs`, `approval_router.rs`, `session.rs`, `relay_client.rs` | `core.rs` (the state machine, with per-channel queues sealed in priority order), `router.rs`, `session.rs`, `relay.rs` | one synchronous core that tests drive without a socket |
| `remote.*` module and `[remote]` policy table | module `anywhere`, settings in the daemon store, never in a project's `policy.toml` (OQ-4, OQ-11) | a project file must not be able to turn the feature on or loosen a limit |
| rekey every hour or 1 GB | enforced at 2^20 messages or 1 GB per direction; the wall-clock hour is not yet | needs the browser to start a new hello on a timer; Phase 2 |
| 10 languages | English and Bangla | the other eight are added with the Phase 2 strings |
| session starts at View after a passkey | same, plus a 2-minute resume for a dropped connection | a network blip should not cost a fingerprint prompt (found while benchmarking) |

What Phase 1 deliberately does **not** claim: the page's JavaScript is served by the relay and is not yet pinned (Phase 4); no Worker CPU figure exists until the Worker is deployed; push and e-mail are Phase 2.

Test inventory: see `CHANGELOG.md` 0.17.0. Run: `cargo test` (sdcd), `pnpm --filter @sdc/cloud test`, `pnpm --filter @sdc/web test`, `pnpm --filter @sdc/web e2e`, `pnpm --filter @sdc/web bench:remote`.

## Phase 2: push, email and the sign-in link (built 2026-10-08)

What was built, and where it differs from the plan.

| Plan | Built | Why |
| --- | --- | --- |
| Live, then Web Push, then email after about 60 s | `Hub.notify` pushes at once when no signed-in browser is connected, and arms the Hub's one alarm; the alarm sends one email for the requests nobody answered; `clear` (the daemon saying it was answered) cancels both. The delay is the daemon's `escalateEmailSec` (1-3600, default 60) | the plan's order, with the 60 s as a setting |
| Push and email carry no details | the push is `{ t: 'approval', url: '/a/<id>' }`, the email says the same in words; the service worker shows fixed wording and ignores any text in a push | the push service and the mail provider are third parties |
| `src/email/` provider adapter | `cloud/src/email.ts`, configured by variables (`EMAIL_PROVIDER` = `resend` or `generic`, `EMAIL_API_KEY`, `EMAIL_FROM`, `EMAIL_API_URL`). Off when any is missing. The key is read once into one request header and is in no log, no response and no error | OQ-14 is still open (below), so the adapter does not guess a vendor's API |
| Device push endpoints in D1 | `push_subscriptions` (already in `0001_init.sql`), written by the Hub when a signed-in device sends `push.subscribe`. Only endpoints on FCM, Mozilla autopush, Apple and Windows push hosts are accepted | an endpoint comes from a browser; without the allowlist the Worker could be made to POST to any address |
| D1 `emails` | set by the **computer** (`email.set`, from the owner's Settings), never by a browser | the computer is the trust root; a browser cannot choose where notifications go |
| Magic link, 10 minutes, once, a button spends it | `POST /api/magic/request` (same answer for any address) and `POST /api/magic/redeem` (an atomic `UPDATE ... WHERE used_at IS NULL`). The secret is in the URL **fragment** (`/m#<computer>.<token>`), so a mail scanner's GET carries nothing and spends nothing; only a hash is stored | threat T5 |
| New device approval after the link | redeeming asks the computer for an ordinary pairing offer (`magic` / `offer` over the Hub). The browser then pairs as with a QR code: six digits, confirmed **on the computer**. The computer answers only if its owner gave an address, no faster than one per 10 s, and never makes a guest offer | the link alone grants nothing, as the plan says |
| Service worker `src/sw.ts` (code pinning, push, preview proxy) | `web/public/sw.js`, plain JS: shows the push and opens the link, no `fetch` handler, no cache. Pinning and the preview proxy stay Phase 4 | scope |

Limits that protect the owner: 60 pushes and 10 approval emails per computer per hour; 5 sign-in links per computer per hour and 10 requests per minute per address (`MAGIC_LIMIT`); at most 100 waiting requests in a Hub.

**Still open.** OQ-14: the provider behind `H:\emailapi.txt` is unconfirmed, so no real email has been sent and no SPF/DKIM/DMARC result has been seen (DNS-RECORDS.md section 6). No real push has reached a real browser: that needs the Worker deployed, the VAPID pair made (`node cloud/scripts/vapid.mjs`) and a phone subscribed. Both are listed in the owner's checklist in `cloud/README.md`.
