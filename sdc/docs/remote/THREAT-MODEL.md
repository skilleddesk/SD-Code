# SDC Anywhere - Threat model (Phase 0)

Scope: the browser <-> Cloudflare Hub <-> PC daemon path and the daemon's reach into local files and VPSs. Based on plan sections 1, 5, 6 and on the code in `sdcd/src/{trust,fs,ssh,pty,agent/gate.rs,auth}`. This is a design-time model; it is reviewed again at the end of each phase and must be backed by the attack tests of plan section 14.

## 1. Assets

| Asset | Where | Harm if lost |
| --- | --- | --- |
| Source code, files on PC and VPSs | PC, VPS | disclosure, tampering |
| VPS SSH keys, passwords, TOTP seeds | PC only (`~/.ssh/sdc_ed25519`, `auth/keychain.rs`) | full VPS takeover |
| Daemon identity key, device public keys, pairing state | PC store / keychain; D1 holds public keys only | impersonation |
| Ability to run commands / approve actions | daemon via Trust Kernel | arbitrary change |
| Audit ledger and checkpoints | PC SQLite (`trust/ledger.rs`, `checkpoints/`) | loss of evidence and rollback |
| Email API key | Cloudflare secret `EMAIL_API_KEY`, local `.dev.vars` | spam / phishing from our domain |
| Account and push endpoints | D1 | notification abuse, metadata leak |

## 2. Actors

| Actor | Capability assumed |
| --- | --- |
| Network attacker | sees and alters traffic outside TLS |
| Malicious or compromised relay (Cloudflare account, Worker, DO, D1 compromise) | sees ciphertext and metadata; can drop, delay, reorder, replay, alter, and serve different JavaScript |
| Email attacker | can read the user's mailbox or a forwarded magic link |
| Thief of an unlocked or locked phone / laptop | has the device |
| Guest-laptop owner | runs a browser the user does not control |
| Malicious file content / prompt injection | text that steers the AI engine |
| Local malware on the PC | out of scope for protection, in scope for not making it worse |
| Compromised VPS | may return hostile output; may try to attack the daemon through SSH output |

## 3. Trust boundaries

1. Browser JS <-> Hub (TLS only; the Hub is untrusted for content).
2. Hub <-> daemon (outbound WSS; untrusted; E2E inside).
3. Daemon <-> Trust Kernel (the kernel is the only gate to files and commands).
4. Daemon <-> VPS (SSH, host key pinned in `ssh/hostkey.rs`).
5. Daemon <-> local loopback services (SDCP on `127.0.0.1:7811`; preview ports).

Trust originates at the **daemon** (pairing is confirmed on the PC, plan 5.1), never at the server.

## 4. Threats and mitigations

Format: threat -> mitigation (plan reference) -> where it is enforced in code -> residual risk. "NEW" = not present in the repo yet.

| ID | Threat | Mitigation | Enforcement point | Residual / test |
| --- | --- | --- | --- | --- |
| T1 | Relay reads content | E2E: HPKE session key, AES-256-GCM, per-direction keys, sequence numbers, rekey (4.1) | NEW `crypto.rs`, `mux.rs` | metadata visible: account, device ids, timing, sizes. Pad frames in a later phase |
| T2 | Relay alters, drops, reorders, replays | AEAD + sequence + `last_seq` resume | NEW `mux.rs`; resume reuses `EventLog::since` | drops/delay = availability loss only. Test: "bad relay" mock (14) |
| T3 | Show A, run B | passkey challenge = `action_hash` of the canonical envelope; daemon recomputes from its own pending request | NEW `envelope.rs`, `approval_router.rs`; request held in `agent/gate.rs` | requires canonical-JSON rigor and one hash definition shared by Rust and TS; golden test vectors |
| T4 | Replay of an approval | nonce store + 5-minute expiry + single use | NEW `crypto.rs` nonce store | nonce store must persist across daemon restart (SQLite), else replay after restart |
| T5 | Email/magic-link abuse | link grants login only, device stays untrusted; one-time; 10 minutes; button press consumes, GET does not (5.1) | NEW `cloud/` | scanners that follow links: GET must be inert (tested). Email provider compromise gives login, not trust |
| T6 | New untrusted device pairs itself | SAS code compared on trusted device; desktop or trusted device approves; `remote.pair` is in `remote_forbidden` | NEW `pairing.rs` | SAS 6 digits: bound attempts (rate limit + lock). Social engineering of the user is residual |
| T7 | Stolen phone | passkey (biometric) needed for Operate/Critical; View locks after 15 min idle; Revoke from desktop; guest = 2 h max | NEW `session.rs` | a phone left unlocked after a passkey prompt is in the Operate window (5 min) |
| T8 | Server serves malicious JS | strict CSP, SRI, code-pinning Service Worker, release hash shown in desktop (Phase 4) | NEW `web/src/sw.ts` | **not fully solvable** in a web app (plan 15). Until Phase 4, the first-load JS is trusted; say so in Settings |
| T9 | Session cookie / token theft | requests signed with a non-extractable device key | NEW `web/src/crypto` | a malicious extension or XSS in the origin can still use the key while the tab lives |
| T10 | Path traversal, symlink escape | daemon canonicalizes; `fs/mod.rs::real_path`, `strictly_inside`, `guard`; VPS `ssh/ops.rs::link_guard`, `guard`; browser only sees opaque `path_id` | existing, extended with `remote.fs.roots` | Windows: 8.3 short names, `\\?\`, junctions, ADS (`file.txt:stream`), case-insensitivity. Add tests on all three OSes (14). VPS-side check relies on the remote `realpath` |
| T11 | Secret file exfiltration | `fs::blocked_reason` hard blocks `.env*`, keys, `.npmrc`, `.netrc`, `credentials`; policy `protected_paths` (DEFAULT_PROTECTED); remote download never allowed for protected | existing, see DESIGN OQ-10 | secrets in ordinary files (`config.json` with a token) are not detected by name; `trust/scan.rs::secrets_in_text` can scan outbound text. Decide whether to scan downloads/opens |
| T12 | Hostile upload | quarantine `.sdc/inbox/`, never executed, size/type limits | NEW `xfer.rs` | chmod: written without execute bit; path = server-generated name, never client-supplied; archives are not auto-extracted |
| T13 | Terminal bypasses Trust Kernel | guarded mode classifies each line; raw mode Critical + recorded (5.6) | `pty/mod.rs::denied_reason_line`, policy `deny_commands/always_ask`, `gate::looks_dangerous`; NEW classifier work | shell syntax can hide intent (`$(...)`, aliases, `eval`, here-docs, `bash -c`, env tricks). Rule: unknown/complex = Critical. `looks_dangerous` is word matching and is not enough alone (DESIGN OQ-9) |
| T14 | Preview used to reach internal services (SSRF) | localhost only, allow-listed ports, no redirects off host | NEW `preview.rs` | DNS rebinding irrelevant because target is a fixed loopback IP; a dev server on an allowed port may itself proxy outward |
| T15 | Remote changes policy / pairs devices / disables remote / reads keychain / erases audit | `remote_forbidden` | NEW `session.rs`; must be hard-coded minimum (DESIGN OQ-12) | if left configurable, an edited list removes the protection |
| T16 | Remote on by accident | `remote.enabled=false` default; toggle only on desktop (1.8) | NEW settings (DESIGN OQ-4) | project `policy.toml` must not be able to enable it |
| T17 | AI prompt injection through a file the user opened or attached | attached files are paths, AI reads them on the PC under the usual gate; all actions pass approval | existing `agent/gate.rs` + policy | injected text can make a *plausible* request the user approves. Cards show the diff and blast radius (WYSIWYS), not the AI's reasoning alone |
| T18 | Hostile VPS output attacks the daemon / UI | output is treated as data; escape terminal control sequences in cards; size caps | `ssh/ops.rs` | terminal escape sequences, huge output, binary; cap and sanitize before sending to browser; browser renders as text, never HTML |
| T19 | Compression oracle (CRIME-style) | compress file chunks only; no compression of frames that mix secrets with attacker-controlled data (7.1.8) | NEW `xfer.rs` | decision must be per-channel; add test |
| T20 | Brute force / DoS on Worker, Hub | rate limits, lock, alert; Cloudflare edge | NEW `cloud/` | DO per account bounds blast radius; unauthenticated endpoints do not touch the DO |
| T21 | Kill abused as DoS | Kill needs no step-up (5.8) and is available at View | `trust/kill.rs::cancel_all` | a stolen unlocked View session can stop work. Accepted; Kill is recorded in the ledger |
| T22 | Device private key read from PC disk | OS keychain | `auth/keychain.rs` | `keychain` is a Cargo feature, default off; Linux uses a file fallback (DESIGN OQ-7). Remote must not silently run with a plain-file identity key |
| T23 | Audit tampering / loss | hash-chained ledger, `audit.verify` | `trust/ledger.rs` | ledger lives on the PC; local malware can truncate the tail. Remote events must be written before the action runs |
| T24 | Email key leak | secret only via `wrangler secret put` and `.dev.vars`; patterns in `.gitignore` and secret-scan CI | Phase 0 files | `H:\emailapi.txt` and the other key files at `H:\` are outside the repo but in plain text; plan 11 says move to a keychain and delete. A bare key with no name is also invisible to some scanners; the CI filename check covers it |
| T25 | Wrong-host action | every action card names host and cwd; `hostId` is part of `action_hash` | NEW envelope; existing `Envelope.host_id` | UI must make the host unmistakable (colour + name) |
| T26 | Time-of-check/time-of-use on file edit | `file_hashes` in the envelope; `fs::hash_file` before write; refuse if changed | `fs/mod.rs`, `ssh/ops.rs` | VPS: hash then write is not atomic; use write-to-temp + rename + compare |
| T27 | Magic-link / OTP phishing of passkey | passkeys are origin-bound; RP ID `sdc.skilleddesk.com` | NEW `cloud/` | changing RP ID invalidates passkeys (DNS-RECORDS.md section 4) |
| T28 | Daemon-side passkey verification mistakes | daemon verifies the WebAuthn assertion itself (RP ID hash, origin, flags, counter, ES256) rather than trusting the server's verdict | NEW `crypto.rs` | the most delicate new code. Use a vetted crate, add test vectors |

### Added during Phase 1 (what the code does now)

| ID | Threat | Mitigation as built | Residual |
| --- | --- | --- | --- |
| T29 | A local process asks the daemon to pair or confirm a device | none that a daemon can enforce: SDCP on `127.0.0.1:7811` is unauthenticated by design, and any local process can already call `permission.resolve` | local malware is out of scope (section 2); a paired device is shown in Settings and in the ledger, and can be revoked |
| T30 | A stranger makes the relay create Durable Objects for ids nobody owns | per-address rate limit on every socket route; a Hub keeps nothing until a daemon proves it holds the key its id is the hash of; browsers get `offline` and nothing is stored | each probe still wakes a Durable Object once |
| T31 | A flood of `open` messages from the relay grows the daemon's memory | at most 128 tracked connections; waiting ones are dropped after 30 s | the relay can still fill the 128 slots and deny service |
| T32 | A session keeps sending bad passkey assertions or wrong signatures | five bad proofs close the session, are logged as `RemoteActivity`; "too late" and "already answered" do not count | a stolen unlocked phone is not slowed by this; Revoke is the answer |
| T33 | A relay adds or edits the reason of a refusal, which the AI receives as the person's instruction | the reason is part of the signed decision string; an edited or added reason fails the device signature | a reason typed by the person is still text the AI will follow: it is the person's instruction by design |
| T34 | A network blip lets someone resume a session | resume is for the same device (its signed hello), at the level it had, for at most 2 minutes, never after Lock, revoke, a protocol error, or for a guest; the Operate deadline is not extended | a thief holding the unlocked phone within 2 minutes of a drop gets the session back, as they would have had it before the drop |

## 5. Plan-level gaps found (to resolve before Phase 1)

1. `remote_forbidden` as editable config (T15) - make the base set a constant.
2. Device identity key storage when no keychain backend (T22).
3. Remote policy location and the `deny_unknown_fields` parser (DESIGN OQ-4).
4. Where "protected but readable" meets the fs hard block (T11, OQ-10).
5. Nonce/replay store persistence across restart (T4).
6. First-load JS trust until Phase 4 (T8): Operate/Critical require a device-key signature the Hub never sees, so a hostile page cannot approve silently; but it *can* display false cards. As built, the browser recomputes each card's hash from the card it shows and refuses to sign a mismatch, which catches a relay that swaps the card but not a page whose own code lies. A card-hash echo on the desktop remains a candidate mitigation.
7. Metadata visibility (T1) should be stated in the user-facing Settings text.

## 6. Honest limits

- No claim of "hack-proof". An attacker who controls both a trusted device and its biometric can act as the user.
- A fully compromised relay can deny service and observe metadata but, by design, cannot read or forge content - except through served JavaScript until Phase 4 hardening exists.
- Compromise of the PC itself defeats everything, including the audit log.
- Cloudflare account takeover is a high-impact event (code served, secrets); use hardware-key MFA on the Cloudflare account.

## 7. Required tests (from plan 14, mapped)

wrong signature, replay, expired, revoked device, hash mismatch, nonce reuse after restart, tampered ciphertext, reordered frames, untrusted device receives nothing, `remote_forbidden` actions refused at every level, path traversal and symlink/junction escape on Windows/macOS/Linux, protected-file refusal, upload lands only in `.sdc/inbox/`, terminal classifier on hostile command corpus, preview to non-allowed port refused, magic link GET inert, relay down leaves desktop SDC unchanged.

## Rejoining with the passkey (added 2026-10-09): a deliberate trade

**Problem.** The device signing key lived only in the browser's IndexedDB. "Clear browsing data" deleted it, and with it the pairing: the owner had to pair again with a QR code. That made the feature depend on a browser setting no one thinks about.

**Change.** At pairing, the passkey (made discoverable, with the computer's id as its user handle) is asked for its WebAuthn `prf` output. That 32-byte secret seals a *vault* (AES-256-GCM, id as associated data): the device's signing key, its device id and the computer's public keys. The relay (Hub, `vaults` table) stores the ciphertext under an id derived from the same secret. After the browser forgets everything, "Rejoin with my passkey" picks the passkey (user verification required), gets the computer's id and the secret, fetches and opens the vault, and the device is whole again. Code: `web/src/crypto/vault.ts`, `web/src/transport/recovery.ts`, `cloud/src/hub.ts` (`vault.put`, `vault.get`). The Rust daemon is unchanged: it sees the same device key as before.

**What the relay learns.** An opaque id and a ~600-byte ciphertext per device. It cannot open it (the key never leaves the phone), cannot tell which passkey the id belongs to, and an altered or swapped blob fails authentication.

**What is weaker, stated plainly.**

| Before | After |
| --- | --- |
| Joining as this device needed the phone's own storage *and* the passkey | Joining needs the passkey (and so the account that syncs it, e.g. a Google account, plus its fingerprint/face/PIN) |
| An attacker with the synced passkey but not the phone could approve nothing: no device key | An attacker with the synced passkey **and** its user verification can restore the device from the relay and connect as it |

The computer's own rules still hold: the restored device starts locked, every unlock and approval needs a fresh passkey assertion, dangerous actions still need it every time, and the owner can remove the device on the computer, which also deletes its vault (`device.remove`, `devices.sync`). A device removed there is denied at the relay and its vault is gone.

**Where it does not apply.** A passkey without PRF pairs normally but cannot be restored (the page says "Not saved for your passkey" and the rejoin button says why). Guest sessions keep nothing. Phones paired before this change have no vault until they are paired again.

**Residual risks to watch.** A compromised Google (or other passkey-sync) account together with the victim's biometric or PIN; a passkey provider that shares PRF outputs across accounts (none known); the vault id is a bearer secret for fetching ciphertext only, which is useless without the PRF output.
