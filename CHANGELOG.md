# Changelog

All notable changes to SDC (Skilleddesk Code) are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) on `sdcd`'s `Cargo.toml` version, which is
what `host.status` reports as the daemon's version.

This file describes what changed, not what is planned. Anything still open is named in
[`sdc/README.md` → What is deliberately absent](sdc/README.md#what-is-deliberately-absent).

---

**Earlier versions (0.4.1 to 0.4.4) are no longer published.** The release pipeline keeps exactly one
release - the newest - and deletes the others when it publishes (`release.yml`, "Keep only this
release"). 0.4.1 to 0.4.3 never rendered a window at all, and keeping them downloadable next to a
working build is a trap rather than a history. The entries below are kept for the record.

## [0.6.2] — the connect screen is not empty any more

0.6.1 was tagged, built and replaced by this one on the same day, so no user ever downloaded it; the
release pipeline keeps exactly one release, the newest. Everything in the 0.6.1 entry below is in this
build, and this is the defect that entry left behind - found by clicking through the build that was
just made, which is the only way it could have been found.

### Fixed — the Provider Hub drew nothing

`provider.list` answers with the eleven provider cards and appends **no** event. 0.6.0 started calling
it at startup, which was right - the 0.6.0 entry says so - but nothing folded the answer: the Hub, the
topbar's plug and the status bar all read the event log's fold, that fold stayed empty, and the screen
showed `0 CONNECTED · None yet` next to a daemon that knew about Claude, ChatGPT, Gemini, five API
providers and Ollama. The one screen that signs a CLI in was therefore empty, which is exactly what was
reported: *"there is nothing at all in the connect module"*. A read whose result nobody keeps is the
same as no read at all.

`withProviders()` folds the answer the way `withWorkspace()` folds `session.list`'s - a list is a
read's result, not a stream of events - and `connectDaemon()` calls it between `host.status` and
`session.list`. `ProviderStatus` is still the event for a *change* (`provider.save`, a CLI login
finishing), so both paths converge on the same array.

`ProviderRecord` also gained the `initial` field the daemon has always sent - the letter in the card's
logo circle - and that the protocol had never declared.

### Verified

`pnpm typecheck`, `pnpm lint`, **28 vitest tests** (one new: the provider list is folded), the bundle
smoke run, and a real window: the Hub lists every card, and clicking Claude starts `cli.login` without
a second click.

---


## [0.6.1] — two chats per click, a ring around every field, and an API key that finally reaches the provider

Three things a person using 0.6.0 reported, and the wall behind the third one.

### Fixed — one click on `New chat` opened two chats

The sidebar drew two rows for one chat, and the tab strip, the count pill and the row list all
disagreed about how many existed. Two independent defects, both of them under the same click:

* **The Tauri bridge could open two notification sockets.** `sdcp_call` connects lazily, and its
  `connect()` guarded on an `AtomicBool` that is only set *after* the await points - so two calls that
  arrive together both find `connected == false` and both reach `subscribe()` at the bottom. `App.tsx`
  starts the boot handshake (`connectDaemon`) and the first heartbeat (`watchDaemon`) in the same tick,
  which is exactly that case, so from 0.6.0 on this happened on **every launch**: every event the
  daemon pushed was emitted twice, and the app folded each one twice. `SdcpBridge` now holds a
  `connect` mutex for the whole connect (the second caller waits and then finds the bridge connected)
  and `subscribe()` is idempotent behind a `subscribed` flag that the reader clears when its socket
  ends.
* **`SessionOpened` was not idempotent.** A replay of the log is legitimate - the bridge asks for
  `event.list since=0` on every connect, and `session.list` may already have listed the session
  between the two - but the reducer appended unconditionally, so the second copy of the same session id
  became a second row *and* a second React key. The reducer now treats a second open of an id known id
  as what it is: the same session.

### Fixed — a bright accent ring followed the caret around the window

Every field you can type in was outlined in `--border-focus` the moment it was focused: the prompt box
and `Filter chats…` through `focus-within`, the modal fields through `focus:border-border-focus`, and
- the part that made it feel like it was everywhere - the global `:focus-visible` ring, because per the
CSS spec a text control matches `:focus-visible` on *every* focus, mouse included. A keyboard ring
that appears when you click with a mouse is not an affordance; it is a border under the cursor. Typing
controls now get a quiet 1px `--border-strong` outline instead, the composed boxes change their border
colour on focus without the glow, and buttons, rows and links keep the 2px accent ring they always had
- for those, `:focus-visible` really does mean reached by keyboard.

### Added — a TLS client, so an API key reaches the provider

`sdcd` links `ureq` (rustls with the webpki roots), and `native_api`'s `post_stream` sends `https://`
instead of refusing it. Before this, an API key could not be used at all: the adapter failed honestly
with "a TLS client is not linked in this build", which is the one sentence a user cannot act on.
Measured against the real endpoints with a deliberately invalid key:

```
anthropic said: API key is invalid. (401)
openai said: Incorrect API key provided: sk-bogus-key. (401)
```

- both requests reached the provider and were understood, which is what those two sentences prove.
With a valid key the same call streams tokens. `Test connection` in the Provider Hub now means it: the
key is used for one read-only call to the provider's own model list (`api.anthropic.com/v1/models`,
`/v1/models` on the OpenAI-compatible providers), and a rejection is shown in the provider's words
(`invalid x-api-key (401)`) rather than as a status code with nothing behind it. `tests/live_api.rs`
holds both of those checks, `#[ignore]`d because CI has no keys.

### Changed — a subscription card signs in on the click that opens it

Clicking Claude, ChatGPT or Gemini in the Provider Hub already opened the real flow - the daemon drives
the CLI's own login (`cli.login`), shows the provider's URL, and takes the code back - but it needed a
second click on `Sign in` to start. It starts itself now, once per open, with a `Try again` button for
a login that stopped and a status line that says `The sign-in stopped before it finished` instead of
leaving `Waiting for the CLI…` under a login that is over.

### Fixed — a host that wants a password was reported as unreachable

`ssh -o BatchMode=yes` cannot answer a password prompt or a one-time verification code, and the daemon
says "did not answer" for the whole family of hosts that ask for one - including a VPS the user logs
into from a terminal every day. `probe_ssh` now reads `ssh`'s own refusal and reports what happened:
the target answered and asked for a password or a code that this call cannot type, with the way out
(a public key) in the same sentence.

### Tests

`sdcd`: 103 unit tests (five new - the two https ones, the key-check shape and its model counting, and
the two SSH refusals), 5 lifecycle, 5 VCR, 2 live checks behind `--ignored`; clippy clean under
`-D warnings`. The window: `pnpm typecheck`, `pnpm lint`, **27 vitest tests** (one new: a replayed
`SessionOpened` stays one row), the bundle smoke run, and a real `SDC.exe` driven over CDP.

---


## [0.6.0] — a server can be removed, and an added one is still there tomorrow

Six defects, all of them found by using the installed build rather than by reading it. They are
listed in the order a person meets them.

### Fixed — a server could not be removed, and added ones piled up

`host.remove` has been declared in `protocol/types.ts`, with its `{ hostId }` param and its
`{ removed: boolean }` result, since the schema was written. The daemon answered `unknown method` and
the UI had no button, so a host added by mistake was permanent.

The sidebar is where the other half of the same defect shows: `host.add` upserted a row for whatever
`user@host` it was given, so adding one machine three times produced four hosts called `Website`, all
of them identical and none of them removable. `session.open` made it worse - it called `upsert_host`
with the literals `Local` / `local` / `connected` for **any** host id, so opening a chat on a VPS
rewrote that VPS's row: its name, its kind and its `target` were replaced.

* **`host.remove` is implemented** (`sdcd/src/sdcp/methods.rs`): the row, its sessions, their turns,
  their checkpoints and their rewind stack go, child-first because `foreign_keys` is ON. The **event
  log is not touched** - it is append-only, and the removal is itself an event (`HostRemoved`) that a
  reconnecting window replays. `local` is refused with a sentence rather than a crash.
* **`HostRemoved` joins the catalogue** (schema `eventTypes`, the TypeScript union and
  `SDCP_EVENT_TYPES`), so the reducer takes the host and its rows off the screen - in this window and
  in the next one.
* **The same `user@host` is one host.** `host.add` asks `Store::host_id_for_target` first and answers
  with the existing id and `reused: true`. The schema and `types.ts` carry `reused`, so a caller can
  say "already in the host list" instead of pretending it connected.
* **`session.open` uses `ensure_host`**, which creates the row only when it is absent, so a VPS keeps
  its name, its kind and its `target`.
* **The sidebar has the button** (`.host-remove`, on hover beside `+`, with a confirmation that names
  what goes with the host). `local` does not get one: a control whose only outcome is an error is
  worse than no control.

### Fixed — adding a server looked like nothing happened

`host.add` pushed exactly one `HostStatus` (`connecting`) and stopped. The dot stayed blue forever,
nothing ever said whether the machine could be reached, and adding the same host again was the only
thing that seemed to do anything - which is how one VPS became four rows.

* **The daemon measures.** After answering, `host.add` runs `ssh -o BatchMode=yes -o ConnectTimeout=8
  -o StrictHostKeyChecking=accept-new <target> true` on a blocking task, then pushes a second
  `HostStatus` (`connected` or `offline`) **and a `Toast`** carrying the sentence that says which:
  `root@vps.example is reachable`, `… did not answer: <first line of stderr>`, or
  `… was added, but ssh is not installed on this machine`.
* **The dialog's own sentence stays honest.** `intents.addHost` says what `host.add` answered
  (`Added prod-1 · checking it can be reached…`) and leaves the verdict to the daemon, because the
  daemon is what ran `ssh`.

### Fixed — an added host was gone on the next launch

Nothing asked the daemon for its host list. `host.add` wrote a row and pushed one event; the window
folded that event and then lost the host the moment it was closed, so the next launch showed only
`local` and every added host looked as though adding it had failed. The row had been in the database
the whole time.

* **`session.list` answers with the tree**: every host with its sessions (`hosts: [{ …, sessions }]`,
  with `target` included). `Store::hosts_with_sessions` builds it. The protocol's `SessionRecord[]`
  result was wrong about what the daemon sends and is corrected to `HostRecord[]`.
* **`connectDaemon()` folds it** at startup through `withWorkspace()` - a state patch, for the same
  documented reason `withDoctorRun` is one: a list is a read's result, and re-emitting an event per
  host on every launch would append a copy of the whole tree to the log each time a window opened.
* **`host.status` records the row it describes**, so the machine this window is on is in the list that
  gets folded.

### Fixed — losing the daemon was silent

`sdcd` is started by the app and killed with it, but it can also die on its own: a crash, a task
manager, a machine under memory pressure. Nothing said so. The window looked normal, every click
answered nothing, and the only way to find out was to notice that nothing happened.

* **A heartbeat** (`watchDaemon`, every 5s) asks `event.subscribe` - a pure read, and deliberately
  *not* `host.status`, which pushes a `HostStatus` event and would therefore append 720 events an hour
  to a log whose whole purpose is to be readable.
* **The answer is presentation, not history** (`store/daemon.ts`): the event log cannot carry "the
  daemon is gone", because the daemon is the log's writer. `misses` counts consecutive failures and
  the copy stops hedging at three.
* **It is said once, in each direction**: one toast when the daemon goes, one when it answers again -
  not one every five seconds, which is how warnings get ignored. The banner above the chat
  (`#degradedBanner`) carries the same news with a `Retry now` button, and it takes precedence over
  the per-host message, because when the daemon is gone that message is noise.

### Fixed — the control reset promised something it did not do

`styles/globals.css` has described itself since 0.5.2 as the reason a control can never paint a light
box in a dark theme - *"no background of their own: a control is a hole in the surface it sits on"* -
and the rule underneath that paragraph never declared a background. 0.5.1 shipped a solid
`rgb(255, 255, 255)` sidebar field; 0.5.2 fixed that one component and wrote the paragraph.

* **`background-color: transparent` is now declared** for `input`, `textarea` and `select`, so a
  component that forgets its `bg-` class cannot reproduce the white box. Measured in a real WebView2
  window running the old build: `#sidebarSearch` computed `rgb(255, 255, 255)` with no `bg-` class at
  all; with this declaration the hole is a hole however the component is written.

### Verification

* `sdcd`: 98 unit tests, **5 lifecycle tests** (one new: a host is added once, is listed with its
  sessions, keeps its `target` through a `session.open`, can be removed, and `local` is refused),
  5 VCR tests; clippy clean under `-D warnings`.
* The window: `pnpm typecheck`, `pnpm lint`, **26 vitest tests** (two new: `HostRemoved` takes the host
  and its chats off the screen; `withWorkspace` folds the list a restart used to lose), the bundle
  smoke run, and a real `SDC.exe` looked at over CDP.

## [0.5.3] — the window says which build it is

A user installed a build, saw the white box from 0.5.1, and reported it unfixed. The box *was* fixed -
0.5.2 measures `rgb(6, 7, 10)` at that pixel, against `rgb(255, 255, 255)` in 0.5.0/0.5.1 - but nothing
in the app said which build was on screen, so the only way to tell a fixed install from an old one was
to trust the release notes. That is the defect this release fixes, on top of the one it documents.

* **The status bar shows the build**: `v0.5.3 · sdcd 0.5.3`, beside the host and before the engine.
  Both halves are shown because they can differ - a new window talking to a daemon left over from an
  older install is exactly the case where the fixed window would look unfixed at the point of use. The
  segment is clickable and copies the pair, which is what a bug report needs.
* The measurement itself is now in the repository as a tool: `_verify/png-pixel.mjs` decodes a PNG
  (no dependency) and prints the colour of named pixels, because a user's report is about pixels and
  a computed style is not a pixel. The white box, in that tool's terms:
  `(130,118) rgb(6, 7, 10) luminance 3%` on 0.5.2 versus `rgb(255, 255, 255)` on 0.5.0.

## [0.5.2] — the white box, the stray focus ring, and three CLIs that could not be found

Three defects, all of them visible in one screenshot of the installed 0.5.1 build.

### Fixed — a white rectangle in a dark window

The sidebar's `Filter chats…` field painted **solid white** (`background-color: rgb(255, 255, 255)`,
measured over CDP) inside its own dark rounded wrapper. The cause is one of those things a stylesheet
has to say out loud: an `<input>` with no background of its own gets the platform's *field* style, and
the component simply had no `bg-` class. The prompt box's textarea had a second, related artefact - a
2px blue ring drawn by the engine on top of the design's own focus ring.

* `styles/globals.css` now declares the token system as the default for `input`, `textarea` and
  `select`: no background of their own (a field is a hole in the surface it sits on), `appearance: none`,
  the token placeholder colour, and `color-scheme: dark` (light under `[data-theme="light"]`) so the
  caret and the engine's own widgets follow the theme. The UA outline is replaced by the design's ring -
  `.search-wrap` and `.prompt-box` already ring on `focus-within`, and the modal fields on
  `focus:border-border-focus`.
* `panels/sidebar/Sidebar.tsx` says `bg-transparent` where a reader is looking.
* **Two new gates in `scripts/smoke-bundle.mjs`**, measured in a real browser: *every form control is
  painted by a token* (no control may paint its own background) and *keyboard focus is visible* (focusing
  the first control must change the paint). Both were proved by taking the fix away: the smoke then
  failed with `input#sidebarSearch [216x18] bg=rgb(59,59,59)` and exit 1.

### Fixed — the daemon could not find a CLI installed by npm

`claude`, `codex` and `gemini` were installed (npm, global) and ran in any terminal, and the daemon
reported all three as **not installed**: the Provider Hub said "install it", the doctor's row said
`fail`, and `engine.start` answered with the missing-program sentence. Every subscription provider was
unreachable on Windows while the user's own shell ran them fine.

Two reasons, both in `std::process::Command`'s rules rather than in the CLIs:

* npm installs a **`.cmd` shim** on Windows; `Command::new("claude")` looks for `claude.exe` and gives up;
* npm also writes an **extensionless `claude`** (a POSIX shell script) beside it, and starting *that*
  fails with `os error 193, %1 is not a valid Win32 application` - so the resolver must try the
  extensions **first**, the way `cmd.exe` and PowerShell do.

`host/program.rs` (new) resolves a name the way the platform's shell does, and wraps a batch file in
`cmd.exe /c`, which `CreateProcess` requires. It is used by the doctor, the engines and `pty.open` -
i.e. by every place that starts a program. Measured after the change: `cli.recipes` answers
`installed=true` for all three.

### Fixed — two recipes were wrong for the real CLIs

With the CLIs finally reachable, the login flows were driven for real (`_verify/cli-logins.mjs`,
which runs the daemon and asks each CLI to sign in). Three findings:

* **Claude Code has a subcommand**: `claude auth login` signs in without a terminal. The recipe pumped
  `/login` into an interactive session instead, which needs a TTY - over the daemon's pipes the CLI
  printed nothing and the flow sat at `waiting_for_url` with no URL to show. Now it answers
  `waiting_for_code` with `https://claude.com/cai/oauth/authorize?…`;
* **Codex prints two URLs** - its own callback server (`http://localhost:1455.`, sentence full stop
  included) and then the approval page. `extract_url` now prefers the `https://` page and trims
  sentence punctuation, so the link the user copies is
  `https://auth.openai.com/oauth/authorize?…`;
* **Gemini CLI needs an auth method before it will start a sign-in** ("Please set an Auth method in
  your settings.json"). The daemon now performs a `prepare` step for that recipe: it merges
  `security.auth.selectedType = "oauth-personal"` (Gemini's own name for *Login with Google*, read out
  of the installed package) into `~/.gemini/settings.json`, keeping every other key the file has, and
  answers `prepared: <path>` so the modal can say what it wrote. `gemini --skip-trust` is passed too,
  because its trusted-folder question blocks a piped run before the sign-in.

**Where each one stands after the change** (measured, `_verify/cli-logins.txt`): `claude` and `openai`
reach `waiting_for_code` with their real approval URLs; `gemini` now starts and reaches
`waiting_for_url`. Completing a sign-in needs the account holder in a browser - that part is not
mine to do, and the daemon never touches the credential the CLI writes.

## [0.5.1] — the daemon claimed three providers nobody had signed in to

0.5.0 removed the demo from the window. Then the window was asked for the provider list, and the answer
showed where the same disease lived one layer down: **in the daemon.**

`provider.list` reported `status: "connected"` for Claude, OpenAI and Gemini on a machine where
`claude`, `codex` and `gemini` were not installed - because the fallback for anything that was not an
API key was `else { "connected" }`. It also shipped an invented spend figure in its own catalogue
(`OpenAI API · Direct API key · $12.40 / $50.00 this month`). A green card for a sign-in that never
happened is the worst kind of wrong in a tool that is supposed to be trusted with a shell.

### Fixed — a status is evidence, or it is `needs-auth`

* The fallback is gone. A provider is `connected` only when there is something to show for it: a stored
  row, a secret in the keychain (`api-key`), the Ollama daemon answering on this machine (`local`), or
  - for a subscription - nothing, because a CLI on `PATH` is not a signed-in CLI. Subscriptions answer
  `needs-auth`, and their detail line names the program: `` `claude` is not installed or not on PATH ·
  install it, then run the environment doctor ``.
* The invented spend is removed from the catalogue, and a test now refuses any `$` in a provider detail.
* New test: `a_subscription_is_never_connected_without_evidence`.

### Fixed — the Provider Hub was empty, so the CLI sign-in was unreachable

* **`provider.list` is now called at startup.** Nothing asked for it, so no `ProviderStatus` event was
  ever pushed on a fresh launch and the Hub - the one screen that runs `cli.login`, i.e. "add Claude
  Code by subscription" - drew nothing to click. `connectDaemon()` calls it beside `host.status`, so
  the cards, the topbar's plug and the status bar are built from the daemon's answer.

### Fixed — two stray marks the first install showed

* The turn meta line drew its `·` separator even when there was no forecast to put after it.
* A turn that ended in `ErrorRaised` promised `Running · totals arrive with the last event`, which is a
  promise a failed turn cannot keep. It says `Failed`.

## [0.5.0] — the demo is gone, and Send does something

This release is the answer to one review sentence: *"it is only a UI, and none of it is connected."*
That was accurate. The window drew a **demo** - three hosts, six chats, nine providers, twelve models,
a fabricated turn stream with token counts and prices - and every number in it was invented, including
one inside the daemon. Nothing the user could click reached `sdcd`, because Send was a toast. 0.5.0
deletes the fiction and wires the two paths that matter: a prompt, and a provider sign-in.

### Removed — every piece of demo content

* **`createInitialState()` is now `EMPTY_STATE`.** The reducer's boot fold used to seed a full fake
  world (`seedEvents()`: 3 hosts, 6 chats, 9 providers, 12 models, checkpoints, console lines, a duel).
  A fresh install therefore *looked* connected, busy and expensive while the daemon behind it was
  answering nothing - and the first thing anyone saw was fiction. `seedEvents` and `strings.seed` are
  deleted; the test that used to fold the demo builds a small log of its own instead.
* **The chat was hardcoded.** `Pane.tsx` rendered `DEMO_TURNS` - the prototype's worked example with
  "Turns 1-6 collapsed · 8,420 tokens · $0.31" - so a real run happened behind a window showing the
  same fiction every time, and the live projections the reducer had all along were never drawn. The
  stream now renders `live.ts`'s mapping of the log's `TurnView`s, and an empty chat says it is empty.
* **`lib/daemon.ts` (802 lines) is deleted**, replaced by `lib/standin.ts` (a browser tab's honest
  answer). The old file was an in-process daemon that *simulated* the whole product: streamed a fake
  answer word by word, invented providers, ran a fake OAuth, raised fake console errors. The stand-in
  answers the protocol's shape and refuses what a tab cannot do, in one sentence that says why.
* **The daemon invented a price.** `TurnStarted` carried a fixed `"~$0.10 – $0.28 forecast"` - a hard
  cost for a turn nobody had measured - and that string was also **byte-corrupted in the source**
  (`â€“`). The field is gone.
* **The preview was a mock.** The Preview tab drew a gradient `Login / src/routes/login.tsx` page under
  a hardcoded `http://localhost:5173/login`, which read as a working preview of a dev server the window
  had never started. It now shows nothing, and says so.
* **The context chips were fixed strings.** `1 file` and `12.4k ctx` were printed under every prompt,
  including an empty one. They are conditional now: a count or nothing.
* **The Doctor and Local tabs printed optimism**: `daemon running` and `Ollama installed · 3 models`
  were hardcoded rows. They show the `host.doctor` rows the daemon actually returned, or the sentence
  that says no probe has run.
* **`strings.seed.tip`** (the startup toast about a seeded project) is replaced by one that is true of
  a fresh window: it is waiting for the daemon.

### Added — Send really sends

* **`sendPrompt`** (`store/intents.ts`): opens a session if the window has none (`session.open`), then
  calls `engine.start` with the tier/engine/model the prompt area is showing. The prompt area's
  `send()` used to clear the box and toast `Sent to claude_code · sonnet` without calling anything.
  A refused call now puts the words back in the box.
* **`TurnStarted` carries `prompt`** (protocol, daemon and reducer). The log holds both halves of the
  conversation, so a window that is reloaded draws the question beside the answer instead of starting
  with a reply. New daemon test: `a_turn_carries_the_prompt_the_user_sent_and_no_invented_price`.
* **The turn stream is the log.** `panels/turns/live.ts` maps `TurnView` → the stream's `Turn`, so
  `TurnStarted`/`ThinkingDelta`/`ToolCall*`/`TurnCompleted`/`ErrorRaised` are what you read. A provider
  that is not installed now shows the daemon's own sentence in the stream
  (`\`claude\` is not installed or not on PATH…`) with its `Fix this` action - which is what the
  installed build does on a machine without `claude`, and is how this release was verified.
* **A real empty state inside a pane** (`Nothing here yet`), the `Turns 1-6 collapsed` line built from
  the real count, and a turn footer that no longer prints a bare `·` while the totals are unknown.

### Verified

Installed from `SDC_0.5.0_x64-setup.exe`, then driven over the WebView's CDP: the window opens with
**no demo content**, one real host, and the turns this machine's daemon has (including the daemon's
honest error for a CLI that is not installed); typing a prompt and pressing Send produces a `YOU …`
message drawn from the log, the turn's own meta line, and the daemon's answer or error. `_verify/live-drive.mjs`
is that check, and it is the closest thing here to "does the button do anything".

### Still absent (unchanged, and named)

The provider sign-in flow exists (`cli.login`, `Connect.tsx`) but the Provider Hub's own subscription
tab still drives the OAuth stand-in rather than the recipe table, so "add Claude Code by subscription"
from inside the app is a `cli.recipes` screen away rather than one click; `models.list` cannot verify a
remote provider without a TLS client; and the agent loop is not written. See
[`sdc/README.md` → What is deliberately absent](sdc/README.md#what-is-deliberately-absent).

## [0.4.4] — the window that was black, and the daemon that would not let go

Two bugs that made an installed build look broken, both found by installing it: the window opened with
nothing in it, and a black console window appeared next to it. Every gate in this repository was green
throughout, which is why the fix comes with a gate that is not about compiling.

### Fixed — the window rendered nothing

* **A store selector returned a new array on every call** (`overlays/Toast.tsx`). Zustand 5 gives the
  selector to `useSyncExternalStore`, which calls it on each commit and compares with `Object.is`; a
  fresh array therefore looked like a change, so React re-rendered, compared, re-rendered - until it
  hit its fifty-update limit (`Minified React error #185`, "Maximum update depth exceeded") and
  unmounted the tree. What is left of an unmounted React tree is an empty `<div id="root">`: a window
  the colour of the theme, with no sidebar, no chat and no error the user can see. The selector now
  returns one joined string, which `Object.is` can compare.
* **This was every build from 0.4.1 to 0.4.3.** The line never changed from the first commit, so the
  daemon, the protocol, the login flow and the model catalogue were all working behind a window that
  could not show them.

### Fixed — a console window opened next to the app

* **`sdcd` is a console program, and Windows gives one a console unless it is told not to.** The bridge
  spawned it with `CREATE_NO_WINDOW` missing, so an installed build opened a black window with the
  daemon's path in its title beside the app window. The pipes were already thrown away, so the window
  was pure noise; it is now suppressed at spawn.

### Fixed — the daemon outlived the app, and the next install met it

* **The app stops the daemon it started.** `sdcd` is a child process, and on Windows a child outlives
  its parent: quitting left a daemon holding port 7811. The bridge keeps the `Child` handle and kills it
  on the window's exit event.
* **`sdcd --idle-exit <secs>`** covers the case where the app does not get to quit - Task Manager, a
  crash, `Stop-Process`. The app passes 8 seconds; a daemon started by hand gets no flag and never
  leaves on its own, because a terminal a person opened is a terminal a person closes.
* **`host.shutdown`** (new method, additive to SDCP 0.1) asks a daemon to stop over the wire, and the
  bridge uses it: a daemon answering on the port whose `sdcd` version is not this build's version is
  asked to stop, and the `sdcd` that ships with this build starts in its place. Without it, installing
  an update while an older daemon was still running meant talking to the older daemon - where methods
  like `models.list` simply answer "unknown method", which is the one failure nobody can diagnose from
  the UI. `sdcp_status` now also reports `appVersion`, `daemonVersion` and `restarted`.

### Fixed — a unix socket that two daemons could fight over

* **The socket's name carries its port** (`$XDG_RUNTIME_DIR/sdc/sdcd-<port>.sock`). It was one fixed
  path, so a second daemon on another port unlinked the first one's socket and bound its own - or lost
  the race between the unlink and the bind and refused to start at all, which is how the macOS CI jobs
  caught it: the new lifecycle tests start three daemons at once. A daemon that cannot bind a *unix
  socket* is now a warning as well (`sdcd` keeps serving loopback TCP, which is the transport the app
  uses), instead of a process that exits.

### Added — the check that would have caught it

* **`pnpm --filter @sdc/app smoke`** (`app/scripts/smoke-bundle.mjs`): serves the built `dist`, opens it
  in the runner's Chromium-family browser through `--dump-dom --virtual-time-budget`, and fails unless
  the app mounted, the shell is in the page, there is text to read and nothing threw. No dependencies
  and no CDP, so it runs on any platform that has a browser; in CI it is a required step before the
  Tauri build, and locally it prints `skipped` instead of blocking a machine without one.
* **`tests/selectors.test.ts`**: the cheap half of the same guard, run by `pnpm test` in a second. It
  reads the sources and fails on a store selector whose result cannot be stable - a `.map(...)` that is
  not joined, an object or array literal - and it contains the exact 0.4.1 line as a case, so the rule
  cannot be quietly relaxed.
* **`sdcd`'s lifecycle tests** (`tests/lifecycle.rs`) start the real binary: a client that sends
  `host.shutdown` ends the process, a daemon started for the app leaves on its own, and one started by
  hand stays. Each test also gets a runtime directory of its own, because the socket's path is derived
  from it. 91 daemon tests became 101 (93 unit, 3 lifecycle, 5 VCR); the frontend's 21 became 23.

### Notes

* The frontend smoke is the release gate that was missing: the packaged app is now opened *as a page*
  before it is packaged, which is one step short of the installer and the right place to catch a
  frontend that cannot mount. The packaged window itself is verified by hand on Windows (WebView2) as
  part of the release checklist - [`sdc/docs/RELEASE.md`](sdc/docs/RELEASE.md), which also says what to
  do about a tag whose jobs failed.

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

