# Changelog

All notable changes to SDC (Skilleddesk Code) are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) on `sdcd`'s `Cargo.toml` version, which is
what `host.status` reports as the daemon's version.

This file describes what changed, not what is planned. Anything still open is named in
[`sdc/README.md` → What is deliberately absent](sdc/README.md#what-is-deliberately-absent).

## [0.4.3] — signing in from the app, and a model list that stays current

Two features, and both come from the same question: what does a person actually have to *do* before SDC
can help them? Sign in to a CLI, or paste an API key and pick a model. Both now happen in the app.

### Added — signing a CLI in, from the app

* **`cli.login` / `.status` / `.code` / `.cancel` / `cli.recipes`.** The daemon starts the CLI's own
  sign-in, reads the URL it prints, and hands back the code the user pastes - so the link is copied
  from a field in SDC instead of fished out of a terminal. Three properties make it honest rather than
  clever:
  * **the recipes are data** (`auth::cli_login::RECIPES`): a CLI that changes its login command is a
    row, not a code change, and `cli.recipes` reports whether each one is even installed;
  * **nothing is auto-opened** and **nothing is stored**: the URL is shown and copied, and the code
    goes to the CLI's stdin. SDC never sees the credential - the CLI writes it to its own store;
  * **the CLI's own output is passed through** (`pty.output`, the new tail method), so a recipe that
    has gone stale is visible in the UI instead of silently failing.
* **`pty.output`** - the output tail of a long-running process. `pty.open` used to be fire-and-forget:
  a process you cannot read is a process you cannot drive, and a login URL is exactly the thing that
  would be stuck in an unread pipe.
* **The Connect modal** (`app/src/modals/Connect.tsx`), opened from a provider card: a copy button, an
  open-in-browser link, a paste-code box, and the CLI's live output underneath.

### Added — the model catalogue as data

* **`models.list` / `models.select`, and `protocol/models.json`.** A row's `source` says where it came
  from - `live` (the provider answered just now), `cache` (its last answer, kept in SQLite with a
  timestamp) or `bundled` (shipped with this build) - because "always up to date" is a claim worth
  showing rather than asserting. `Refresh` asks the provider again; a refresh that cannot reach it
  says so in `notes` and **keeps the list**.
* **A model a provider has that this build has never heard of is still listed**, with its price left
  empty rather than guessed. No code change is needed when a provider ships a new model: that is the
  point of the design, and the catalogue itself is a JSON file rather than a table in the source.
* The Connect modal's API half: the key, `Save`, `Refresh`, the list with its source badges, and
  `Use` to record the choice (a setting, not an event - which model is *selected* is a UI preference).
* The browser's in-process daemon answers the same shapes and says plainly, in its `note`, that it is
  the bundle and that a tab cannot drive a CLI.

### Verified

* 91 daemon tests (86 unit + 5 VCR), clippy clean, typecheck/lint/build/test clean.
* **43/43 live smoke checks**, including the sign-in mechanism driven end to end through a stand-in
  CLI (URL found → code pasted → `authenticated`), the catalogue's sources, and a failed refresh that
  explains itself.
* The sign-in recipes are **not** exercised against the real `claude`/`codex`/`gemini` on this machine:
  none of the three is installed here, and the doctor says so. What is proved is the daemon's half -
  the stand-in CLI in the tests behaves like the real ones (prints a URL, waits for a line, announces
  success). The recipes are one table to correct if a CLI changes; the output the UI shows is what
  tells you.

## [0.4.2] — the execute step, and installers for every platform

### Added

* **`shell.run` — the daemon's execute step.** One command, run to completion with both streams
  captured, its exit code reported, and a non-zero exit translated into a plain sentence
  (`{ "title", "explanation", "rule", "fixable" }`). This is the piece an agent loop needs in order to
  *act* rather than describe, and it is what makes SDC more than a viewer of someone else's agent:
  * a **deny list** refuses the handful of commands that destroy a machine (`rm -rf /`, `mkfs`,
    `format c:`, `diskpart`, `shutdown`, `reboot`, `dd if=`, a fork bomb, `git push --force`) with the
    reason attached, matched at the *start* of the line so `echo "rm -rf /tmp"` is not a false
    positive, and one level into a shell (`sh -c "…"`) so wrapping does not hide it;
  * a mutating run writes a **checkpoint before it starts**, and the run is announced as a tool call,
    so the turn stream shows it the way it shows an engine's own tool calls;
  * a **timeout** stops it, and the answer says `timedOut` instead of hanging a turn for ever;
  * output is capped at 256 KB per stream (`truncated: true`) so a chatty command cannot fill the log.
  The module says plainly that this is **not a sandbox**: the permission gate and the checkpoint are
  the real protection.
* **Cross-platform releases in CI** (`.github/workflows/release.yml`). No host can build another
  platform's bundle, so the workflow builds each on its own runner — Windows (NSIS + MSI), macOS
  (Intel and Apple Silicon DMGs), Linux (AppImage + deb) — runs the same gate a developer runs, and
  attaches everything to one GitHub Release. Push a `v*` tag and all six installers exist.
* `pnpm daemon:package` (`app/scripts/package-daemon.mjs`) builds `sdcd` in release and copies it under
  the target triple Tauri's `externalBin` expects, so every installer carries the daemon. It honours
  `SDC_TARGET_TRIPLE` for a cross build.
* `pnpm app:check` (the Tauri bridge's `cargo check`) and `sdcd:test` workspace scripts.

### Changed

* `beforeBuildCommand` is now `pnpm build && pnpm daemon:package`, so a build can never produce an
  installer without a daemon in it - on any machine, CI included.

### Added — the installers

* `pnpm tauri:build` produces an installable app: `SDC_0.4.2_x64-setup.exe` (NSIS) and
  `SDC_0.4.2_x64_en-US.msi`, both carrying the daemon.
* **`sdcd` is bundled as a sidecar** (`bundle.externalBin`), produced by `pnpm daemon:package`
  (`app/scripts/package-daemon.mjs`: builds the daemon in release and copies it under the target
  triple Tauri expects). Before this, an installed app would have opened a window where every call
  failed; now double-clicking the installer yields an app that runs its own daemon.
* **The window connects on mount.** `App.tsx` calls `host.status` once when it mounts, so a fresh
  install starts the daemon and folds *this* machine's real status into the store (engines, keychain
  backend, event count) instead of showing the seed until the first click. If the daemon does not
  answer, the app says so once, in plain words (`strings.daemon.offline`).
* The app's own version moved from `0.0.1` to `0.4.2` (`tauri.conf.json`, both `Cargo.toml`s,
  `package.json` files), so what the About tab shows and what Windows has installed agree.
* `generate.mjs` now ships next to the VCR fixtures, so a clone can regenerate them
  (`node sdc/sdcd/tests/vcr/generate.mjs`) without the machine-local `_verify/` harness.
* `_verify/sdcp-smoke.mjs` — a dev-only harness (outside the workspace) that starts a real `sdcd`,
  exercises every method with realistic parameters, restarts the daemon on the same database and
  checks that the log and the rows survived. 38 checks now, and it is what found the broadcast bug
  below.

### Fixed

* **Every notification now reaches every client, not only the one that asked.** `sdcd` pushed an
  event only to the connection whose request caused it, so the app's dedicated subscribe socket
  (`sdcp_subscribe` → `sdcp://event`) received **nothing**: in desktop mode no turn stream, no
  `HostStatus`, no toast would ever have arrived, while the browser's in-process daemon worked. The
  daemon now keeps one `Fanout` registry of subscribers (`sdcp::notifications`) and broadcasts every
  event to all of them, dropping a client that has gone away. A regression test asserts it, because
  the path had no test before - which is why the bug survived to a smoke run.
* **`provider.remove` actually removes.** It answered `{removed: true}` and did nothing; it now
  deletes the secret from the keychain, puts the row back to `available` and pushes the
  `ProviderStatus` event the Provider Hub's card is folded from.
* **`checkpoint.restore` actually restores.** It answered `{restored: 1}` and did nothing; it now
  finds the checkpoint by id and runs the same restore as a rewind, answering with the number of
  turns it stepped back.
* **`event.append` actually appends.** It answered the sequence number it never used; it now records
  the client's event in the log and broadcasts it, which is what "the log is the state" means for a
  UI-only fact.
* **`provider.test` no longer claims more than it did.** It said `OK · key valid` for any non-empty
  string, without contacting anything. It now answers a `verified` flag - `true` only for the local
  Ollama daemon (a real probe) - and the `detail` sentence says the provider was not contacted when
  it was not. The two modes differ on purpose: the demo daemon asserts the happy path
  (`verified: true`), the real daemon tells the truth.

### Known

* The Rust crates are not rustfmt-formatted. `cargo fmt --check` reports every file, so the
  formatting change is deliberately left out of feature commits and is its own pull request; see
  `CONTRIBUTING.md`. `cargo clippy -- -D warnings` is clean and is the gate that runs today.

## [0.4.1] — the daemon, steps 12–17

The step where `sdcd` stopped being a protocol sketch and became the daemon the app had been written
against, and where the Tauri bridge closed the last gap between them.

### Added — the daemon's engines (spec §11)

* `engines/` — five adapters behind one `trait Engine` (`id`, `start`, `cancel`, `status`) and an
  `EngineRegistry`.
  * `claude_code` — `claude --include-partial-messages`, which the spec makes **mandatory**: without
    the flag the CLI buffers the whole answer and the app's streaming turn would be a comfortable
    fiction. A test asserts the flag is present.
  * `codex`, `gemini` — the same shared CLI adapter (`engines/cli.rs`) with their own program and
    structured-stream flag, so the three cannot drift apart.
  * `native_api` — builds an Anthropic `messages` request or an OpenAI `chat/completions` request,
    parses both SSE dialects, and takes the key from the keychain (never from disk). It streams
    against `http://` endpoints today; a remote `https://` endpoint reports that a TLS client is not
    linked rather than pretending a turn started.
  * `ollama` — real NDJSON against `127.0.0.1:11434`, with `GET /api/tags` backing the Local flow's
    doctor rows.
* `engines::parse_stream_line` / `collect_stream` — the only place a CLI's field names appear, which
  is what lets the VCR fixtures hold all three CLI adapters to one contract.
* `engine.start` now answers with a `turnId` **immediately** and runs the turn on its own task:
  `TurnStarted`, `ThinkingDelta`, the tool-call trio, the `TurnDelta` stream, and `TurnCompleted`
  with the engine's own summary and cost.
* `engine.cancel` / `engine.kill` kill the child process; `engine.status` reports the adapter's real
  state.

### Added — the daemon's remaining modules (spec §10)

* `fs/` — the file guard. `.env` (and `.env.*`), `*.pem`, `*.key`, `id_rsa`, `id_ed25519`,
  `credentials`, `.npmrc`, `.netrc` and the daemon's own data directory are **refused**, not warned
  about, and a blocked name is filtered out of a listing as well as out of a read. Every read and
  write returns the file's SHA-256, which is the `filesHash` a checkpoint stores.
* `git/` — a shadow repository per project (`--git-dir` under the data directory, `--work-tree` the
  project), so the user's own `.git` is never read or written, plus a per-session worktree.
* `pty/` — long-running processes under one registry, killed on shutdown, with `tty: false` reported
  honestly rather than a terminal that is not there.
* `auth/keychain` — the OS keychain behind the `keychain` feature, and a 0600 file fallback that
  `backend()` reports as `"file"` so the doctor can say so.
* `host/doctor` — the ten environment checks of spec §9.10 (Node, Claude, Codex, Gemini, Ollama,
  ripgrep, Port 3000, disk, git, ssh) as real probes with `ok`/`warn`/`fail` and a `fix` label.
* `providers/` — the nine-provider catalogue, the twelve-model registry and the six Provider Hub
  flows' server side. A key goes to the keychain; only its mask reaches SQLite.
* `checkpoints/` — a checkpoint per mutating turn, written **before** the mutation. Its screenshot
  half is a contract plus a path (the app's WebView owns the pixels) and the module says why.
* `rewind/` — both halves of a rewind: the files from the shadow repository, and the conversation
  onto a rewind stack that `rewind.redo` pops.
* `duel/` — two engines on one prompt. `plan()` refuses a one-engine duel; `Keep` archives the loser,
  `Keep neither` archives both, and nothing is ever deleted.
* `errors/translator` — the differentiator: a tool's wall of text becomes one plain sentence, a next
  step and a `fixable` flag (eight rules, from `missing-program` to `budget`).
* `session_bridge/` — switching engines mid-turn by replaying the *conversation*, not the tokens.
* `console/` — the preview console bridge and the one parser for a WebView's two error shapes
  (`at LoginForm.tsx:42:11`).

### Added — the Tauri bridge (spec §5.3, §5.4)

* `app/src-tauri/src/sdcp.rs` — `sdcp_status`, `sdcp_connect`, `sdcp_call`, `sdcp_subscribe` and
  `sdcp_stop_daemon`, plus the `sdcp://event` push. `sdcp_call` takes the envelope the frontend
  already writes (`{ method, params, id }`) and returns the daemon's **whole response**, because
  `TauriTransport` correlates by id.
* Lazy connect: the first `sdcpCall` is what connects — and, if nothing answers on `127.0.0.1:7811`,
  starts `sdcd` (next to the app binary, then `sdcd/target/{release,debug}`) and waits for the port.
  A daemon the user started by hand is never started twice and never stopped.
* Loopback TCP rather than a Windows named pipe, stated at the top of the module: the daemon already
  serves newline-delimited JSON on every platform, and a named pipe would be a second, Windows-only
  server inside `sdcd` for no gain the user can see.

### Changed

* **The daemon persists the event log as it appends.** `EventLog::append` writes through to SQLite
  instead of a per-request batch in the connection loop, so an event a background turn pushed is
  durable even if the client that started the turn has gone away.
* **`engine.start` makes its session exist.** A turn may arrive for a session the daemon has not seen
  (the app seeds its chats in the UI), so `Store::ensure_session` creates the row the foreign key
  needs instead of failing the turn.
* **A missing CLI is a sentence, not a stack.** `cli.rs` reports "`claude` is not installed or not on
  PATH…" and the translator's `missing-program` rule turns it into the card's title and explanation.
* `event::duel_started`, `duel_resolved`, `tool_call_started`, `tool_call_output`,
  `tool_call_completed` and `error_raised` were added to the event catalogue, so every event the
  daemon emits now carries its `type` field.

### Fixed

* `host.doctor` reports the database's real path and size and the event count, instead of a sentence
  that named neither.
* Two keychain tests shared one entry name and raced under the parallel test runner; each test now
  uses its own name.
* `git::checkpoint` stages the *project's* files into the shadow repository (`--work-tree`), which is
  what makes `git.status` and `git.diff` answer about the user's tree rather than an empty one.

### Tests

* `sdcd`: 65 tests, including `tests/vcr.rs` — thirteen fixtures in `sdcd/tests/vcr/` (twelve
  conversations plus `native-13-user-pastes-screenshot.jsonl`, the vision path) replayed through the
  same parser the daemon uses for that engine, and compared with the kinds each fixture declares.
* Verified live, over TCP, on Windows: `host.doctor` returns ten real checks; `engine.start` answers
  `{"turnId":"turn-1"}` and then streams `TurnStarted` → `ErrorRaised` → `SessionUpdated`, with every
  event persisted.

## [0.3.0] — the event store, the modals and the keyboard

* `protocol/sdcp.schema.json` and `protocol/types.ts` — one protocol, one set of types.
* `app/src/store/` — an event-sourced store: `reducer.ts` is pure with the clock in the event, and the
  seed is a fold of the log, so the UI cannot hold state the log does not.
* `app/src/modals/` and `app/src/overlays/` — Provider Hub, Settings (seven tabs), Add host,
  Permission, Palette, Search, Toast, New chat and the F1 keymap reference.
* `app/src/commands/registry.ts` — one registry (Global 10, Session 6, Model 2, Approval 5,
  Timeline 6, Actions 3) feeding the palette, the F1 reference and Settings → Keymap, so they cannot
  disagree (principle P7).

## [0.2.0] — the UI

* The full UI against `design/ui-prototype.html`: topbar, sidebar, tab strip, turn stream, prompt
  area, right panel (Context, Console, Time Machine, Duel), status bar and the token layer.
* `app/src/strings.ts` as the single home for user-visible copy (spec §2.7), and no hardcoded colours
  (spec §8.1).

## [0.1.0] — the shell

* Tauri 2 window (`SDC`, 1280×800, identifier `dev.skilleddesk.sdc`), the five-region `#app` grid,
  the token layer, `pnpm` workspaces and the platform-narrowed bundle targets.
* The Windows `.msi` was built and its metadata read back (`ProductName = SDC`).


[0.4.2]: https://github.com/skilleddesk/SD-Code/releases/tag/v0.4.2
[0.4.1]: https://github.com/skilleddesk/SD-Code/releases/tag/v0.4.1
[0.3.0]: https://github.com/skilleddesk/SD-Code/releases/tag/v0.3.0
[0.2.0]: https://github.com/skilleddesk/SD-Code/releases/tag/v0.2.0
[0.1.0]: https://github.com/skilleddesk/SD-Code/releases/tag/v0.1.0

