# Remote hosts — how SDC reaches a VPS (0.7.13)

> **A host that will not connect?** [`SSH-CONNECT.md`](SSH-CONNECT.md) is the operational companion to this
> file: every step's real code, the exact `ssh` command to run by hand for each layer, and a table of the
> sentences the daemon says (`did not answer: … SDC dialed port 22`, `did not present a host key: …`,
> `answered, but it does not accept SDC's key yet`) with the cause and the fix for each. This file is the
> *why*; that one is the *where does it stop*.

The report this document answers, word for word: *"vps connect korai jasse nah. VPS connection ar
jonno je sokol layer proyojon sai rokom kono kisui aikhane nai."* 0.7.0 gave `host.add` a parser, a
key and a probe, and that was one layer of the eight a remote host needs. This file is the research,
the layer map, and the rules the code in `sdcd/src/ssh/` follows.

---

## 1. How VS Code Remote-SSH actually works (the research)

Read against the official documentation (`code.visualstudio.com/docs/remote/ssh`,
`/docs/remote/troubleshooting`, `/docs/remote/faq`).

| Piece | What VS Code does | Why it matters here |
| --- | --- | --- |
| The transport | The **local** `ssh` binary is the only thing that talks to the host. The extension never speaks SSH itself. | The key, the `~/.ssh/config` and the known-hosts rules stay the ones the user already has. SDC follows the same rule: `ssh` is spawned through `host::program::command`, so a Windows `ssh.exe` resolves like every other program. |
| The far side | A **VS Code Server** is installed into `~/.vscode-server/bin/<commit>/` and started there. It needs a glibc Linux (`kernel ≥ 3.10`, `glibc ≥ 2.17`, `libstdc++ ≥ 3.4.18`), `tar`, a writable `$HOME` and `/tmp`. | An editor on the far side is the *only* way to make file watching, search and language servers fast — not an implementation detail. SDC's equivalent is a second `sdcd`; see §5. |
| The channel | The server binds a port on the remote's loopback and the client reaches it through an **SSH tunnel** (`ssh -L`). The docs, literally: *"All other communication between the server and the VS Code client is accomplished through the authenticated, secure SSH tunnel."* | The remote must allow local forwarding (`AllowTcpForwarding yes`), and **no port is exposed to the internet**. SDC's protocol already supports this shape (`WebSocketTransport`, `VITE_SDCP_URL`); what is missing is provisioning, not the wire. |
| Host key | Verified by `ssh` against the user's `known_hosts`. A **changed** host key is refused with an error, and the user is asked to verify the fingerprint before updating `known_hosts`. | This is the layer SDC was missing. `accept-new` accepted *any* key on first contact, silently, with no way to see or re-pin a fingerprint. 0.7.13 pins, shows, and refuses a changed key. |
| Auth | Keys *or* password *or* 2FA. The UI asks and hands the answer to `ssh`; nothing is stored. | SDC uses a key it owns, installs it once with the password the user already has (through a PTY), and stores neither. |
| Requirements | Local OpenSSH client; a remote `sshd` with `AllowTcpForwarding`; a shell; a writable home. | SDC's doctor list gains no new hard requirement: `ssh` is already checked, and the remote needs only a shell — no `sdcd`, no Node, no `tar`. |

Two conclusions from reading it, and they are the design of this release:

1. **The security boundary is the host key, and it is a *pin*, not a formality.** VS Code does not
   quietly trust a machine it has never seen; it shows a fingerprint and asks. Anything that
   automates the *first* connection (`StrictHostKeyChecking=accept-new`, `=no`,
   `UserKnownHostsFile=/dev/null`) trades that away.
2. **Everything else is layers, and each one is separately visible.** Connect, trust, look, read,
   write, run. A "VPS connect" that only authenticates is a connection to nothing.

## 2. What SDC had, and the four things it did not

0.7.0 shipped (`sdcd/src/auth/remote.rs`): the target parser (`ssh -p 8443 user@host`), SDC's own key
at `~/.ssh/sdc_ed25519`, one password-driven `authorized_keys` install through the daemon's PTY, and a
probe sentence. Real, and about one eighth of the feature:

* **the port was not persisted.** `host.add` stored `parsed.user_host` in `hosts.target` and dropped
  the port — so a VPS added on `8443` connected once (the probe used the parsed target) and every
  later connection would have used 22. *This was the "vps connect korai jasse nah" bug wearing a
  different hat.*
* **the host key was never verified.** The probe ran with `StrictHostKeyChecking=accept-new` against
  the **user's own** `known_hosts`, so the first key ever seen was trusted for ever and a *changed*
  key was invisible in the UI.
* **nothing worked on the far side of the connection.** `fs.*`, `git.*` and `shell.run` all resolve a
  path on the machine the daemon runs on. A host was a row and a dot: no folder, no file, no branch.
* **the app never asked for a remote host.** `VITE_SDCP_URL` existed for a *global* websocket; a chat
  on a VPS and a chat on the laptop were indistinguishable to every file call.

## 3. The layers, and where each one lives

| # | Layer | Where | What it answers |
| - | ----- | ----- | --------------- |
| L1 | **Target** | `auth::remote::parse_target` (0.7.0) | `ssh -p 8443 mehedi@1.2.3.4` → `{user_host, port}`. The port travels with every call. |
| L2 | **Key** | `auth::remote::{ensure_key, public_key, install_key}` | `~/.ssh/sdc_ed25519`, `-N ""`, `-c sdc`. The password is used once, into a **pinned** host only, and stored nowhere. |
| L3 | **Trust** | `ssh::hostkey` (0.7.13) | Scan the key the host presents, print its `SHA256:` fingerprint, pin it in SDC's own `known_hosts`, refuse a changed one, and re-scan before pinning so the fingerprint the user confirmed is the fingerprint that is stored. |
| L4 | **Connection** | `ssh::Ssh` (0.7.13) | One hardened argument set for every call: `BatchMode`, `ConnectTimeout`, `ServerAlive*`, `IdentitiesOnly`, `PreferredAuthentications=publickey`, `PasswordAuthentication=no`, `StrictHostKeyChecking=yes`, `UserKnownHostsFile=<data>/ssh/known_hosts`, `-i <sdc key>`. |
| L5 | **Files** | `ssh::ops` + `fs.*` (0.7.13) | `fs.list` / `fs.read` / `fs.write` / `fs.stat` / `fs.search` take a `hostId`; on an SSH host they run over `ssh` and return the **same shapes**, so the tree, the Preview editor and the Diff button need no second implementation. The file guard applies to remote paths too (`.env`, `*.pem`, `id_rsa`, `credentials` are refused **and counted** as hidden). |
| L6 | **Shell and git** | `ssh::ops`, `shell.run`, `git.status`, `git.diff` | `git.status` answers for the *project's own* repository on that machine (`("", 0)` when there is none), `git.diff` is `git -C <root> diff`, and `shell.run` runs one command in the remote's shell. |
| L7 | **The folder a chat works in** | `project.add` + `session.projectRoot` (0.7.6, remote since 0.7.13) | `project.add { hostId: <vps>, root: '/srv/app' }` validates the folder *on that machine* (`test -d`), so a typo is refused by the host that has the folder rather than by the laptop that does not. `session.list` already carries `projectRoot`, so the tree follows the chat. |
| L8 | **The window** | `modals/AddHost.tsx`, `panels/ui/status.ts`, `store/intents.ts` | The Add-host dialog grows the trust step (fingerprint + **Trust and connect**), the host dot knows a fifth state (`untrusted`), and every `fs.*`/`git.*` call carries the chat's `hostId`. |
| L9 | **The engines, on the host** | `engines/cli.rs::remote_command`, `ssh::ops::{turn_line, kill_line}` | A chat whose folder is on a host runs its CLI *there* — same prompt placement, same JSON stream, same events — with the pid in a file so `engine.cancel` is a `kill -TERM` rather than a closed pipe. |
| L10 | **Checkpoints and rewind, on the host** | `checkpoints::Snapshot`, `rewind::apply`, `ssh::ops::shadow_*` | The shadow git repository lives at `$HOME/.sdc/git/<hash of the root>` **on that machine**, so a checkpoint commits there and a rewind checks the host's files out of it. |

## 4. The security rules (and why each one is a rule)

1. **A password is only ever typed into a host whose key is pinned.** `host.add` scans with
   `ssh-keyscan` (a key exchange and **no authentication**); if the key is unknown the install does
   not run and the password is dropped. It is used later, by `host.trust`, after the pin is written.
2. **A changed host key is an error, not a question.** If a pinned host presents a different key, the
   daemon refuses with both fingerprints. Nothing offers "continue anyway" — that is the exact message
   a man-in-the-middle attack needs.
3. **`IdentitiesOnly=yes` + `-i <sdc key>` + `PreferredAuthentications=publickey`.** Without
   `IdentitiesOnly`, OpenSSH offers the agent's keys to whatever host it is pointed at; a VPS is
   somebody else's machine, and it should learn only the key minted for it.
4. **SDC keeps its own `known_hosts`** (`<data>/ssh/known_hosts`, `0600`, directory `0700`). The
   user's file is neither read for a decision nor written with SDC's pins: a pin SDC took is SDC's to
   explain, and a host a person trusts in their terminal is not automatically trusted by a daemon.
5. **The file guard runs on the remote path too.** `fs::blocked_reason` is a name rule, so the same
   function refuses `.env` on `/srv/app` and on `H:\app`, and the count is reported (`3 names hidden`)
   rather than the rows silently missing.
6. **No shell interpolation of anything the user typed.** Every path and argument that reaches a
   remote shell goes through `ssh::sh_quote` (single quotes, `'` → `'\''`), so a folder called
   `$(rm -rf /)` is a folder called `$(rm -rf /)`.
7. **Nothing about a host is stored that is not a pin**: no password, no key material, no agent
   forwarding (`-A` is never passed), and `port` and `host_key` are the only new columns.

## 5. The layer that runs on the far side — and the one it deliberately does not have

Every operation in §3 is an **`ssh` command**, including the ones that used to be impossible on a host:

* **the engines** (`engines/cli.rs`). A chat whose folder is on a host starts its CLI *there*: the child
  the daemon spawns is an `ssh` whose remote command is `cd <folder> && sh -c 'mkdir -p …; echo $$ >
  <pid>; exec env … <cli> …'` — the prompt still travels on stdin, the CLI's own JSON stream still arrives
  on stdout, so the parsing, the events and the window are unchanged. The whole command is wrapped in
  **`setsid`** when the host has it (`if command -v setsid …`, decided on the host, in the same round
  trip), which makes the CLI the leader of a new **process group** — so `engine.cancel` runs
  `kill -TERM -<pid>` over a second `ssh` (then `-KILL` a second later) and stops the *tree*, not one
  process. That is the difference between stopping a turn and leaving the test runner it spawned to
  finish on somebody's server;
* **long-running processes too** (`pty.open` with a `hostId`). The Terminal's `Run in background` is an
  `ssh` on the near side and the same `setsid`+pid-file line on the far side, so `pty.output` reads the
  process's output tail, `pty.write` reaches its stdin (that is what an `ssh` forwards), and `pty.close`
  signals its **process group** there instead of only dropping the connection — the kill is fired on a
  detached thread so a Stop button never waits on a slow link;
* **checkpoints and rewind** (`ssh::ops::shadow_*`). A remote project gets a shadow git repository **on
  its own machine** — `$HOME/.sdc/git/<hash of the root>`, the same derivation the local `git::shadow_path`
  uses — so `git.checkpoint`, `checkpoint.create`, `shell.run`'s pre-command checkpoint and the
  per-turn `CheckpointSaved` commit there, and `rewind.apply` restores the host's working tree with
  `git checkout <sha> -- .` on that host. The `Snapshot` enum in `checkpoints::mod.rs` is the seam:
  no call site can forget which machine it is talking about, because the compiler asks;
* **the doctor** (`host::remote_checks`). `host.doctor { hostId }` answers about *that* machine: is it
  reachable, is its key the pinned one, does it have `git`/`claude`/`codex`/`gemini`, is `$HOME`
  writable, and (when a chat is named) does its folder exist there.

`pty.open` accepts either a program (`command`+`args`, which is how `cli.login` starts a CLI) or a whole
**line** (`line`), which is what a terminal hands it: here the local platform's shell runs the line
(`sh -c` / `cmd /C`), and on a host the **host's** shell does. The line is checked statement by statement
(`pty::denied_reason_line`) before anything starts, so `git status && shutdown /s` is refused exactly as a
bare `shutdown /s` would be.

What is deliberately absent, and why — each with the condition that would change the answer:

**1. A second `sdcd` on the host, reached over an `ssh -L` tunnel.** §1 shows VS Code taking this route,
and the question "why doesn't SDC?" deserves the table rather than a sentence. What a far-side daemon
would add, verified against what this release can now do:

| What a tunnel + second `sdcd` buys | Where it stands after 0.7.13 |
| --- | --- |
| A **PTY** on the host | Done without it: `pty.open { line, hostId }` is an `ssh` whose remote process writes a pid file, so output, stdin and *cancellation* all work. The one thing missing is a real tty (`-tt`), which belongs to a terminal *emulator* — a different feature, not a different architecture. |
| **File watching** on the host | Nothing in the window watches files *locally* either. Adding it means a watcher (a dependency, or a long-lived `inotifywait` per folder) on both sides; the tunnel would only change *where* it runs. It is a feature with its own design, not a remote-layer gap. |
| A **cache**/warm state on the host | Not used by anything the window calls today. Every method in the schema that touches a host answers from a fresh `ssh` command, and the two that would benefit most (`fs.list`, `git.status`) are already the cheap ones. |
| A second **event log** to reconcile | A cost, not a benefit: the daemon's log is append-only and replayed to the window, and two logs would need a merge rule for every fact (a checkpoint on the host, a turn streamed from it, a `HostStatus` about it). |
| An `sdcd` **artifact per architecture** | Only CI can produce one, and only a release can ship it — a provisioning path SDC cannot verify from inside the app is a promise it should not make. |

So the exec route is not a shortcut around the tunnel: for everything the window can currently express,
the two are **equivalent**, and this one needs nothing installed on the host. The condition that would
change the answer: the first feature that genuinely needs a *far-side resident process* — live file
watching with near-side latency, or a terminal emulator with a real tty — would pay for the artifact,
the port, the token and the second log. Until then, `ws` transport stays in the protocol for exactly that
design, unused on purpose.

**2. SDC does not install anything on a remote host.** Adding a host needs its shell, and a chat needs one
of the CLIs (`claude`, `codex`, `gemini`) — so on a host without one, the doctor row says
`not installed` and an engine that is asked to run anyway fails *in the CLI's own words*. What changed in
this release is what the row's **`Install`** button does: it opens the **Terminal on that host** (focusing
one of its chats first, so the command runs in *that* folder on *that* machine) and the person runs the
installer themselves. The command is theirs; the guard rails are SDC's — the deny list, the checkpoint
before it runs, and the tool-call pair in the session's log. There is deliberately no one-click
`npm i -g …` over `ssh`: it writes to somebody's server as their user, and the honest version of that
button is a terminal, not a silent install.

**3. `fs.search` on a host needs `grep`** (every Linux and macOS box has it; a stripped container might
not, and then the row says so in `ssh`'s own words).

## 6. The Terminal surface — and what it is not

0.7.13 adds the panel's seventh tab: **Terminal**, next to the Console, because the two are the panel's two
logs (the Console is what the *page* said, the Terminal is what a *command* said). The prototype in
`design/ui-prototype.html` carries the surface and its CSS, so the window is not the first place it was
drawn; the tab strip's switch is generic (`data-panel` / `data-panel-view`), so the prototype's Terminal is
live like the other six.

What it does:

* a line runs through **`shell.run { line }`** — the same call the engine's `run` step uses — so a typed
  command is a step in the chat: the daemon checkpoints first (the command may change files), announces
  the tool call, applies the deny list, and on a host runs it *there*, in that chat's folder;
* **`Run in background`** uses `pty.open { line, hostId }`, and its output keeps arriving while you look at
  another tab (`watchBackground` polls `pty.output` from `App.tsx`, not from the tab — a tail that only
  advances while it is on screen is a tail that lies about what it collected);
* the line's **`where`** is printed above the input, always: `~/app/landing on prod-1`. That one sentence is
  the reason the tab exists at all in a remote-capable app: `rm -rf build` reads the same on a laptop and on
  production, and nothing else in the window makes the difference that visible;
* a refusal shows up in the entry's **stderr slot** — where a terminal puts a reason — with the daemon's own
  sentence (`\`git push --force\` was refused because it rewrites published history`), not a stack trace.

What it deliberately is not, in this release:

* **not a terminal emulator.** `tty: false` is in every answer: there is no pty, no `-tt`, no escape
  sequences. A full-screen program (`vim`, `top`, `htop`) needs one, and that is a different feature with
  a different dependency;
* **not interactive after it starts.** A foreground line runs to completion (or to its timeout) and the
  input is busy while it does. `pty.write` exists and a background process can be fed on stdin over the
  `ssh` — but the tab offers no stdin UI yet, so "answer the prompt" is not a flow this release pretends
  to have;
* **one background process at a time**, and ↑ recalls one line. Both are choices about a 400px column
  rather than limits of the daemon.

## 7. Files and tests

| File | Role |
| ---- | ---- |
| `sdcd/src/ssh/mod.rs` | `Ssh` (the hardened argument set, `run`, `run_with_stdin`), `SshOutput`, `sh_quote`. |
| `sdcd/src/ssh/hostkey.rs` | `scan`, `fingerprint`, `known_hosts_path`, `Trust`, `pin`, and the base64/SHA-256 fingerprint OpenSSH prints. |
| `sdcd/src/ssh/ops.rs` | The remote operations — `list`, `read`, `write`, `stat`, `search`, `git_status`, `git_diff`, `shell`, `is_dir`, `home`, `writable_home`, `probe`, the turn line and its pid file (`turn_line`, `raw_line`, `kill_line`, `wrap_line`), and the shadow-git (`shadow_checkpoint`/`shadow_diff`/`shadow_restore`). |
| `sdcd/src/engines/cli.rs` | Where a turn becomes an `ssh` (`remote_command`), and where a cancel becomes a **process-group** kill. |
| `sdcd/src/pty/mod.rs` | `denied_reason` and its line form (`denied_reason_line`, `shell_for_line`), and the long-running registry: `open(command/line, …, label, remote)` / `output` / `write` / `close`, where `close` signals the host's process group when the process is remote. |
| `sdcd/src/checkpoints/mod.rs`, `sdcd/src/rewind/mod.rs` | The `Snapshot` seam: `Unbound` / `Local(root)` / `Remote(ssh, root)` — the compiler asks at every call site. |
| `sdcd/src/host/doctor.rs` | `remote_checks`: the host's own environment, including the `Trust`/`Re-pin` rows. |
| `sdcd/src/sdcp/methods.rs` | `host.add`, `host.trust`, `host.key`, `host.doctor`, and the `hostId` routing in `fs.*`, `git.*`, `shell.run`, `project.add`, `engine.start`. |
| `sdcd/src/store/sqlite.rs` | Migration `0003-host-ssh` (`port`, `host_key`), `host_type` (`ssh` → `vps`), and the readers/writers for both columns. |
| `protocol/{sdcp.schema.json,types.ts}` | `host.trust`, `host.key`, the `untrusted` status, `detail`/`hostKey` on `HostStatus`, `hostId` on the file methods, `sessionId` on `host.doctor`, and the Terminal's two forms — `shell.run.line` and `pty.open.line`/`hostId`. |
| `app/src/modals/AddHost.tsx` | The trust step and the host card: the fingerprint, `Trust and connect`, `Re-pin`, and the host's own doctor rows. |
| `app/src/modals/HostSwitcherPopover.tsx` | The way back to a host's key and environment, from the switcher, without re-adding anything. |
| `app/src/panels/right/TerminalTab.tsx` | The Terminal surface: `where`, the runs, `Run` / `Run in background` / `Stop` / `Clear`, and the refusal slot. |
| `app/src/store/terminal.ts` | The tab's own state: entries (capped), the ↑ history, the one background process, `busy`. |
| `app/src/store/intents.ts` | `terminalSubject`, `runCommand`, `runInBackground`, `pollBackground`, `stopBackground`, `watchBackground` — and the `hostId` on every file/git/shell call. |
| `docs/SSH-CONNECT.md` | The same chain **operationally**: each step's real code, the exact `ssh` command to check it by hand, and the failure table. Start here when a host will not connect. |
| `_verify/ssh-doctor.mjs` | One command, five layers, outside the app: client, port, host key, SDC's key, probe — the same binaries SDC uses, with exit code 1 when the probe fails. |
| `_verify/probe-remote.mjs` | Drives the daemon's real methods against a live SSH server: `host.add` → `host.key` → `host.trust` → `host.doctor` → `ssh.key` → `fs.list` → `git.status` → `shell.run {line}` → `pty.open`/`output`/`close`. |

Tests: `cargo test --manifest-path sdcd/Cargo.toml` covers quoting, fingerprinting, known-hosts parsing,
the listing/read/write parsers, the trust decision table, the remote turn line (port, folder, pid file and
args-inside-the-line, plus the `setsid` group and the group-kill escalation), the line guard
(`refuses_a_denied_program_anywhere_in_a_line`) and the shell a line becomes; `protocol/check.mjs` keeps the schema, `types.ts` and the daemon's dispatch in
step; the app's `vitest` run covers the intent paths, the status map, the pin and the host's doctor.
`_verify/probe-remote.mjs` is the live one: it drives `host.add` → `untrusted` → `host.trust` → the probe
against a real server (`node _verify/probe-remote.mjs root@github.com 7899` against a daemon started with
`--port 7899 --database <temp>`), and prints every `HostStatus` so the sentences and the fingerprint can be
read as they arrive.

