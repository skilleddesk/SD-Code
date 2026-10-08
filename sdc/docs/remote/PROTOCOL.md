# SDC Anywhere - wire protocol (0.17)

What travels between a browser, the relay and `sdcd`. The code is the authority: `sdcd/src/anywhere/{crypto,core,router,session,webauthn}.rs`, `web/src/{crypto,transport}/`, `cloud/src/hub.ts`. Shared byte-level vectors are `protocol/remote-vectors.json` (checked by Rust and TypeScript). SDCP itself (the desktop protocol) is unchanged except for the ten `anywhere.*` methods and the `RemoteActivity` event in `protocol/sdcp.schema.json`.

## 1. Three hops

```
browser ──WSS──►  relay Worker + Hub (Durable Object, one per daemon)  ◄──WSS (daemon dials out)──  sdcd
         /c/<id>?device=<device id>          forwards JSON, reads nothing             /d/<id>
         /p/<id>   (pairing, no auth)
```

* `<id>` = base64url of the first 16 bytes of SHA-256 of the daemon's identity public key (22 characters). Nobody else can claim an id: the Hub checks the key hashes to it.
* Browser sockets must carry `Origin: <WEB_ORIGIN>`. The daemon is not a browser and sends none.
* Nothing is opened on the PC. The daemon only connects out.

## 2. Hub handshakes (the relay's own authentication; it protects the relay, it does not create trust)

| Role | Hub sends | Peer answers | Hub then |
| --- | --- | --- | --- |
| daemon | `{"t":"challenge","nonce"}` | `{"t":"auth","pub","sig"}`, `sig` = ECDSA-P256 over `sdc-anywhere/v1/hub-auth\|<nonce>\|<id>` | pins `pub` on first contact, `{"t":"ready"}` |
| device | `{"t":"challenge","nonce"}` | `{"t":"auth","sig"}`, over `sdc-anywhere/v1/hub-device-auth\|<nonce>\|<id>\|<device>` with the key the daemon registered | tells the daemon `{"conn","open":{...}}`, `{"t":"ready","now"}` |
| pair | nothing | `hello` only (at most 3 messages, 16 KB) | forwards to the daemon |

A socket that has not authenticated in 15 s is closed. Unknown device and bad signature get the same answer. 300 messages/s per socket, then it is closed. Daemon → Hub control: `devices.sync`, `device.add`, `device.remove`, `notify`, `clear`, `ping`. Hub → daemon: `{"conn","open"}`, `{"conn","msg"}`, `{"conn","close"}`.

## 3. The end-to-end session (Hub cannot read or alter it)

HPKE (RFC 9180) base mode: DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, AES-256-GCM. One context per direction; an HPKE context numbers its own messages, so a drop, repeat or reorder fails to open. Frames carry the number as associated data (8 bytes big-endian).

```
hello   { t, device, enc, ct, sig }    ct = Seal(HelloPlain)      sig = ECDSA(device key, "sdc-anywhere/v1/hello" | enc | ct)
welcome { t, enc, ct, sig }            ct = Seal(WelcomePlain)    sig = ECDSA(daemon key, "sdc-anywhere/v1/welcome" | hello.sig | enc | ct)
frame   { t:"f", n, ct }               ct = Seal(JSON {ch, type, id?, body}), aad = n
```

HPKE `info`: `sdc-anywhere/v1/c2d|<id>|<device>` (device → daemon) and `.../d2c|...`. `HelloPlain = {device, pk, ts, nonce, last_seq, pair?}`. The daemon checks: the device is registered and not revoked, the signature (before opening anything), `ts` within ±2 minutes (the relay supplies its clock to the browser in `ready`), the nonce unused (persisted, so a restart does not reopen the window). Rekey = run the handshake again (limit 2^20 messages or 1 GB per direction).

**Resume.** A session that drops (network, relay restart) and returns within 2 minutes comes back at the level it had, with the same deadline. After an explicit lock, a revoke, a protocol error, or 2 minutes, the passkey is asked for again.

## 4. Levels (`session.rs`)

`locked` (sees only a count; can Kill) → `view` (passkey; locks after 15 min of no use) → `operate` (passkey; 5 minutes) ; `critical` is not a state: it is what one action asks for, answered by a passkey assertion over that action's hash. Guests are `view` only, at most 2 hours, no resume. Settings can shorten these, never lengthen.

Tunnelled SDCP calls (`rpc`) go through an **allow-list**: `kill.all kill.list` (any level), `host.status session.list event.list audit.verify checkpoint.list trust.score` (view). Everything else is refused, and `anywhere.*`, `policy.*`, `keychain.*`, `provider.key*`, `secrets.*`, `host.password`, `host.shutdown`, `ssh.key`, `team.set`, `remote.*`, `audit.erase`, `reset.erase` are refused by name even if someone adds them to the list.

## 5. Messages (channel `control` unless noted; priority control > pty > stream > fs > preview > xfer, applied before sealing)

| Direction | `type` | `body` |
| --- | --- | --- |
| → | `capability.challenge` | `{level: "view"\|"operate"}` → reply `{challenge, rp_id, expires_at}` |
| → | `capability.unlock` | `{assertion}` (WebAuthn, challenge = `SHA-256("sdc-anywhere/v1/unlock" \| level \| nonce)`) |
| → | `capability.lock` | |
| ← | `capability.state` | `{level, operate_until, guest}` |
| ← | `approval.requested` | `{envelope, action_hash}` (only to view/operate sessions) |
| ← | `approval.pending` | `{count}` (locked sessions: no content) |
| → | `approval.decision` | `{request_id, decision, action_hash, device_sig, assertion?}` |
| ← | `approval.resolved` / `approval.expired` | `{request_id, ...}` |
| → | `control.kill` | none; works while locked |
| → | `stream.subscribe` (`last_seq`) | replays up to 500 events after `last_seq`, then live `stream.event {seq, event}` on channel `stream` |
| → | `rpc` | `{method, params}` → reply from the daemon |
| → ← | `ping` / `res` | `res`: `{id, ok, body \| error{code,message}}` |

### Approvals

`action_hash` = SHA-256 of the canonical JSON (RFC 8785 restricted: sorted keys, no whitespace, **integers only**) of the envelope including `"v":1` and a random `nonce`. `decision` is `allow_once`, `deny`, or `deny:<reason>`; `device_sig` = ECDSA over `"sdc-anywhere/v1/decision" | action_hash | decision-string` (so a reason is signed). The daemon compares `action_hash` with its own copy (constant time) before anything else. Allow needs the operate window; `DANGEROUS` risk, or action `delete`/`deploy`/`rewind`, needs a fresh passkey assertion whose challenge is `action_hash`. The first valid answer wins; the gate is released with `agent::gate::resolve`; the ledger records `PermissionResolved` with the device name and the reason.

### Pairing

QR/link: `/pair#v1.<token>.<identity pub>.<kem pub>` (base64url; the fragment never leaves the browser). The device pins both keys, makes a non-extractable ECDSA key and a passkey, and sends a first `hello` carrying `pair = {token, name, user_agent, sign_pub, passkey_id, passkey_pub, assertion}` where `assertion` signs `SHA-256("sdc-anywhere/v1/pair-challenge" | hello.nonce)`. The daemon spends the token only after the signature, nonce and passkey proof check out, then shows `SAS = digits(SHA-256("sdc-anywhere/v1/sas" | token | device key | daemon key))` as `123 456`; the browser shows the same. Trust begins when the person confirms **on the computer**. Guests have no passkey.

## 6. What the relay can and cannot do

Can: see who is connected and when, message sizes and timing, that a request exists (for the push/email nudge), drop or delay traffic, refuse service. Cannot: read or alter a frame undetected, replay or reorder one, make a device trusted, make the daemon accept an approval, or learn a command. The page's own JavaScript is served by the relay: until Phase 4 (code pinning), a hostile relay could serve a page that *displays* something false; it still cannot produce a valid signature without the device key. See THREAT-MODEL.md T8.
