<div align="center">

<img src="app/src-tauri/icons/128x128@2x.png" alt="SDC — Skilleddesk Code" width="96" height="96" />

<h1>SDC — Skilleddesk Code</h1>

<p><strong>Local-first coding-agent workbench.</strong></p>

<p>
  <sub><strong>PROPRIETARY SOFTWARE — ALL RIGHTS RESERVED. NOT OPEN SOURCE.</strong><br />
  Published for reading, review and security audit only; no licence to copy, modify or<br />
  redistribute is granted. See <a href="../LICENSE">LICENSE</a>.</sub>
</p>

</div>

---

This repository is a **monorepo**: a Tauri 2 desktop app, a host daemon written in Rust,
and the protocol definition that connects them.

> **Status — the app and the daemon both run, and they talk to each other.**
> The app is complete: topbar, sidebar, tabs, turn stream, prompt area, right panel, status bar,
> command palette, Provider Hub (six flows), Settings (seven tabs), Search, Add host, Permission,
> Time Machine, Duel and the F1 keymap reference — all folded from one append-only event log
> (spec §3.3). The daemon implements the whole plan of spec §10: SDCP over newline-delimited JSON,
> five engine adapters (`claude_code`, `codex`, `gemini`, `native_api`, `ollama`), the file guard,
> a shadow git repository, processes, the keychain, the ten environment checks, the provider
> backend, checkpoints, rewind, duel, the error translator, the Session Bridge and the console
> bridge. The Tauri bridge (`app/src-tauri/src/sdcp.rs`) is what carries SDCP into the window.
> What is still open is named, not hidden, in
> [What is deliberately absent](#what-is-deliberately-absent).

| Document | Role |
| --- | --- |
| `docs/MASTER_SPEC.md` | Master build specification v3.0. Locked stack (§4.1), UI law (§7–§9). Where it and the prototype disagree, the prototype wins (§7). |
| `SETUP.md` | Fresh-machine toolchain guide (Node, pnpm, Rust, Tauri CLI, platform dependencies). |
| `design/ui-prototype.html` | Working UI prototype — the UI source of truth. |
| `design/tokens.json` | Design tokens extracted from the prototype CSS. |
| `protocol/README.md` | What SDCP is, and the rules this directory will follow. |
| `docs/REMOTE.md` | How a host is reached: the Remote-SSH research, the eight layers of 0.7.13 with the file each lives in, the seven security rules, the Terminal surface and what it is not, and what is deliberately absent — each with the condition that would change the answer. |
| `docs/SSH-CONNECT.md` | The same chain **operationally**: every step's real code, the exact `ssh` command to run by hand for each layer, and the failure table (`did not answer` / `did not present a host key` / `does not accept SDC's key yet`) with its cause and its fix. Start here when a VPS does not connect. |
| `docs/ROADMAP-v4.md` | **Proposal, awaiting approval** — the v4 direction: the gap analysis against the owner's asks, seven architecture decisions with their proof, the phased plan (SDC Agent, Verify pipeline, connected-only models), and the UI proposal drawn in `design/ui-proposal-v4.html`. |

## Layout

```
sdc/
├── app/                     Tauri 2 desktop app
│   ├── src/                 React 18 + TypeScript (strict) frontend
│   │   ├── layout/              the shell: region placeholders, Shell.css, viewport + keyboard hooks
│   │   ├── store/               Zustand stores — layout.ts is the first one
│   │   ├── styles/globals.css   design-token layer, Tailwind layers, base typography
│   │   ├── strings.ts           every user-visible string (spec §2.7)
│   │   └── App.tsx              composition root: #app, .workspace, the five regions
│   ├── src-tauri/           Rust shell (window, plugins, capabilities, bundling)
│   ├── package.json         @sdc/app — frontend scripts + Tauri CLI
│   ├── vite.config.ts       Tauri-driven dev server (port 1420)
│   ├── tailwind.config.ts   token aliases; no hardcoded colours (spec §8.1)
│   ├── postcss.config.js    Tailwind + autoprefixer
│   ├── tsconfig.json        strict type checking for src/
│   ├── tsconfig.node.json   strict type checking for the build tooling
│   ├── eslint.config.js     ESLint 9 flat config
│   └── index.html           Vite entry document
├── sdcd/                    Host daemon — a separate Rust binary (spec §3.1)
│   ├── src/main.rs
│   ├── src/ssh/             reaching another machine (0.7.13): mod.rs (flags, run), hostkey.rs (pins), ops.rs (its files)
│   └── Cargo.toml
├── protocol/                SDCP schema (JSON Schema + generated TS types) — README only in STEP 1
├── design/                  tokens.json (authoritative), ui-prototype.html
├── docs/                    MASTER_SPEC.md, REMOTE.md, RELEASE.md
├── package.json             workspace root: scripts that fan out to app/ and sdcd/
├── pnpm-workspace.yaml      the workspace definition
└── README.md
```

## Toolchain

Everything needed is installed step by step in `SETUP.md`. Short version:

| Requirement | Version | Note |
| --- | --- | --- |
| Node.js | 20 LTS (target) | Newer versions work; the locked target is 20 LTS. |
| pnpm | 10.x | The workspace manager (see below). |
| Rust | stable, via rustup | Pinned by `rust-toolchain.toml`. `app/src-tauri` and `sdcd` are both cargo projects. |
| Tauri CLI | 2.x | Installed as a dev dependency of `app`, so `pnpm tauri:dev` needs no global install. |
| Windows only | MSVC Build Tools + WebView2 | Installed on this machine: VS 2022 Build Tools with MSVC 14.44.35207, Windows SDK 10.0.26100.0 and WebView2 153. Tauri cannot link a Windows build without them. |
| Linux only | `webkit2gtk-4.1`, `libayatana-appindicator3` etc. | See `SETUP.md` §6. |

## Why pnpm workspaces (and not a Makefile)

The master spec mentions `make gen` as the token generator, and STEP 1 had to choose between a
root `package.json` workspace and a Makefile. **pnpm workspaces won**, for four reasons:

1. The acceptance command is `pnpm --filter app …`, which only exists in a pnpm workspace.
2. pnpm is already the locked package manager, and `pnpm-workspace.yaml` gives one lockfile for
   every JavaScript package (today `app/`; `protocol/` joins it in STEP 2).
3. `make` is not part of the locked stack and is not present on a stock Windows machine, while
   `pnpm` is required on all three target platforms anyway.
4. Rust targets do not need `make`: `cargo` is already a task runner. The root scripts simply
   forward to it (`pnpm sdcd:run`, `pnpm rust:clippy`).

The token generator that the spec calls `make gen` therefore becomes a pnpm script
(`pnpm tokens:gen`) in STEP 2, with the same contract: `design/tokens.json` →
`tokens.css` + Tailwind theme. Nothing else changes.

## Commands

Run these from the repository root unless noted. `pnpm --filter app …` and
`pnpm --filter @sdc/app …` are equivalent (`app` is the package directory, `@sdc/app` its name).

| Command | What it does |
| --- | --- |
| `pnpm install` | Install all workspace dependencies. |
| `pnpm dev` | Vite dev server on <http://localhost:1420> (browser only — no Tauri window). |
| `pnpm build` | Type check + production frontend bundle into `app/dist`. |
| `pnpm tauri:dev` | **Opens the desktop window** (Tauri dev: Vite server + Rust shell). |
| `pnpm tauri:build` | Release build + installers for the current platform. |
| `pnpm typecheck` | Strict TypeScript check of `src/` and of the build tooling. |
| `pnpm lint` | ESLint over the frontend. |
| `pnpm sdcd:run` | Build and run the host daemon (`cargo run` in `sdcd/`) — serves SDCP on `127.0.0.1:7811`. |
| `pnpm test` | Vitest over the frontend: the reducer, the intents (the daemon-facing half, including the Terminal's), the command registry and the store-selector rule (91 tests). |
| `pnpm --filter @sdc/app smoke` | **Opens the built `dist` in a real browser** and fails unless the app mounted, the shell is in the page, there is text to read and nothing threw. No dependencies, no CDP; `skipped` on a machine with no Chromium-family browser, required in CI. |
| `pnpm sdcd:test` | `cargo test` in `sdcd/`: 192 tests — 175 unit, 9 daemon-lifecycle (real binary: `host.shutdown`, `--idle-exit`, hand-started stays), 2 streaming and 6 VCR — plus 2 `--ignored` live tests that need a real provider. |
| `pnpm daemon:package` | Build `sdcd` in release and stage it as the sidecar the installer bundles. |
| `pnpm rust:fmt` / `pnpm rust:clippy` | Format / lint the daemon crate. |

Inside `app/`: `pnpm tauri dev`, `pnpm tauri build`, `pnpm tauri icon <png>` also work directly.
Inside `sdcd/`: plain `cargo run`, `cargo test`. Inside `app/src-tauri/`: `cargo check` type checks the
bridge without building the window.

## Run it

Two ways, and they differ only in who answers SDCP.

**1. Browser (fastest, no Rust build).** `pnpm dev` opens <http://localhost:1420>. `lib/sdcp.ts`
finds no Tauri bridge and no `VITE_SDCP_URL`, so it starts the in-process daemon
(`app/src/lib/daemon.ts`), which answers the *same* envelopes over `LoopbackTransport`. Every flow —
the six Provider Hub flows, Settings, Search, Palette, Time Machine, Duel — is live, because the
fallback is a daemon and not a mock: it appends to the same event log the reducer folds.

**2. Desktop (`pnpm tauri:dev`).** The window loads, `TauriTransport` is chosen, and the first
`sdcpCall` runs `sdcp_call` in `app/src-tauri/src/sdcp.rs`, which connects to `127.0.0.1:7811` —
starting `sdcd` first if it is not already running (it looks next to the app binary, then in
`sdcd/target/{release,debug}`). Everything the daemon pushes comes back as the `sdcp://event` Tauri
event, so a turn's `TurnDelta` stream reaches the store after `engine.start` has already answered.

**Who owns the daemon.** The app does, for the daemon it started: it spawns `sdcd` with
`CREATE_NO_WINDOW` (a console program would otherwise be given a console window next to the app), it
kills that child when the window closes, and it passes `--idle-exit 8` so a daemon whose app was killed
rather than closed still leaves. A daemon started by hand — `pnpm sdcd:run`, or a terminal you are
watching — has no such flag, is never killed, and never leaves on its own.

Two consequences worth knowing:

* **An older daemon on the port is replaced, not used.** `host.status` reports the daemon's `sdcd`
  version; when it is not this build's, the bridge sends `host.shutdown` (a real method, additive to
  SDCP 0.1), waits for the port and starts the `sdcd` this build ships. Installing an update over a
  running daemon therefore just works; the alternative was every method answering `unknown method`.
* **`sdcp_status` tells you what is connected**: `appVersion`, `daemonVersion`, `restarted`,
  `spawned`, `lastSeq`.

**Which engine answers.** `engine.start` looks the engine up in `EngineRegistry`:
`claude_code` (`claude --include-partial-messages`), `codex`, `gemini`, `native_api` (an `http://`
endpoint streams today; a remote `https://` one reports that a TLS client is not linked in this
build) and `ollama` (real NDJSON against `127.0.0.1:11434`). A CLI that is not installed produces a
`ErrorRaised` that says so, with the doctor's `Install` row behind it — which is what the translator
rule `missing-program` exists for.

## Acceptance (STEP 1)

| Criterion | Command | State |
| --- | --- | --- |
| Window shows "Hello SDC" | `pnpm --filter app tauri:dev` | **Verified** — window titled `SDC`, client area 1280×800 (measured 1296×839 including native decorations), showing "Hello SDC" with the Lucide terminal icon and the mono tagline on the `--bg-base` / `--bg-raised` tokens. 389 crates compiled in 1m46s. |
| Daemon smoke line | `cargo run` inside `sdcd/` | **Verified** on the default MSVC toolchain — exits 0 and prints `sdcd ok`. (Also verified earlier with the gnu toolchain, before MSVC existed.) |
| Strict TypeScript passes | `pnpm --filter app typecheck` | Verified — exits 0. Also verified: `pnpm --filter app lint` and `pnpm --filter app build`. |
| Windows installer builds | `pnpm --filter app tauri:build` | **Verified** — `target/release/sdc.exe` (8.81 MB) and `bundle/msi/SDC_0.0.1_x64_en-US.msi` (4.26 MB), with WiX reporting `Finished 1 bundle`. Installer metadata read back: `ProductName = SDC`, `ProductVersion = 0.0.1`, `Manufacturer = skilleddesk`, `ARPPRODUCTICON = ProductIcon`. |
| No feature code | — | Verified by inspection; see below. |

## Build configuration decisions

**Window (spec §7.2, task 3).** `src-tauri/tauri.conf.json` sets `title: "SDC"`, 1280×800,
minimum 900×600, resizable, `decorations: true`, `transparent: false`, centred. The window label
is `main`, which is what the capability file targets and what the frontend API refers to.

**Identifier and bundles (task 4).** Bundle identifier `dev.skilleddesk.sdc`; bundle targets
`["msi", "dmg", "appimage", "deb"]`. Because no host can build another platform's installer,
each platform narrows the list through Tauri's platform config files:

| File | Effective targets |
| --- | --- |
| `tauri.conf.json` (base) | `msi`, `dmg`, `appimage`, `deb` — the full cross-platform intent |
| `tauri.windows.conf.json` | `msi` |
| `tauri.macos.conf.json` | `dmg` |
| `tauri.linux.conf.json` | `appimage`, `deb` |

These files are merged over the base config automatically, so `pnpm tauri:build` works on every
host without passing `--bundles` by hand.

**Capabilities (task 4, minimal).** `src-tauri/capabilities/default.json` grants exactly three
permission sets to the `main` window: `core:default` (Tauri's own safe defaults), `shell:default`
(open `http(s)://`, `tel:`, `mailto:` links with a pre-configured scope) and `dialog:default`
(message/open/save dialogs). The `fs` plugin is **not** installed and **not** granted —
filesystem access arrives with explicit, reviewed scopes, never as a blanket allow. Registering
a plugin in Rust grants nothing on its own; the capability file is the only grant.

**CSP.** `app.security.csp` is `null` in STEP 1 (the Tauri template default) so that HMR is not
fighting a policy while there is no remote content to protect. Tightening it is a release
blocker and belongs with the fs scopes.

**Icons.** `src-tauri/icons/` holds a generated placeholder set (background `--bg-base`, accent
tile `--accent`, "SDC" wordmark): `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`
(512, for Linux bundles), `icon.ico` (16–256, BMP entries) and `icon.icns`. Every artefact was
generated from one 1024 px master and read back to confirm it loads. Replace the whole set with
`pnpm --filter app tauri icon design/icon.png` once a real logo exists.

**TypeScript strictness.** `strict: true` plus `noUnusedLocals`, `noUnusedParameters`,
`noFallthroughCasesInSwitch`, `noImplicitOverride`, `noImplicitReturns`,
`verbatimModuleSyntax`, `isolatedModules` and `forceConsistentCasingInFileNames`. Build tooling
(`vite.config.ts`, `tailwind.config.ts`) is type checked separately in `tsconfig.node.json` with
Node types instead of DOM types. `exactOptionalPropertyTypes` and `noUncheckedIndexedAccess` are
deliberately off in STEP 1 — cheap to add per-module later, expensive to retrofit by surprise.

**Tailwind 3, pinned.** The locked stack requires `tailwind.config.ts`; Tailwind 4 is CSS-first
and has no config file, so `tailwindcss ^3.4` is pinned. Components take colour from token
aliases whose values are CSS custom properties, declared in `src/styles/tokens.css` (spec
§8.1–§8.4, mirrored 1:1 from `design/tokens.json`) and imported first by `globals.css`. The token
namespace is part of the class name, so a class spells both the property and the role:
`bg-bg-raised`, `text-text-primary`, `border-border-subtle`, `text-state-error`, `bg-diff-addBg`.
The generator (`pnpm tokens:gen`) is still to be written; until then the two token files are kept
identical by hand and must change together.

**Tauri CLI.** The CLI is a dev dependency of `app/` (`@tauri-apps/cli`), so `pnpm tauri:dev`
works without a global install. `cargo tauri` (the route `SETUP.md` describes) also works if the
binary is on `PATH`; both read the same `tauri.conf.json`.

**Release profile.** `app/src-tauri/Cargo.toml` builds releases with `lto`, `codegen-units = 1`,
`opt-level = "s"` and `strip`. If release build time becomes a problem, relax `lto` first.

## What is deliberately absent

Nothing here is a stub that pretends: each gap below is a boundary the code states at the point where
it is reached.

* **A hardened VPS is reached by signing in once (0.9.0), and the code is asked for, never stored.** A host
  that only offers `keyboard-interactive` (password + verification code per connection) is signed in
  through one kept-open connection (`sdcd/src/ssh/session.rs`, ControlMaster + `-O proxy`, 10-hour
  persist); every later call rides it. What is deliberately absent: the daemon never stores the password
  or a code, so after the persist window (or a reboot) the person signs in again - that is the design,
  not a gap. Windows needs the `ssh` Git for Windows ships (multiplexing); without it the key-install
  path of 0.7.13 remains the fallback and the doctor says so.
* **SDC is an agent now, for API and local models (0.8.0) - and it stays the workbench around the CLIs.**
  With **Agent** chosen, a native-API or Ollama model runs inside the daemon's own loop (`sdcd/src/agent`):
  eight tools on the daemon's existing `fs.*` / `shell.run` / `git.*` paths, so it works in a folder on a
  host exactly as it does here; a permission gate that waits for the person (Simple / Pro / Auto); a
  checkpoint taken synchronously before its first change; 25 steps per turn. It was verified against a real
  provider: a DeepSeek turn edited a file, ran `npm test` and reported 5 of 5 passing.
  What it deliberately is not: a replacement for `claude` / `codex` / `gemini`. A subscription is only
  usable through its own CLI, and those CLIs are agents already; the switch changes nothing for them. What is
  still narrow: the loop has had one provider family exercised live (OpenAI-compatible); the Anthropic
  dialect is covered by a recorded stream, not yet by a paid live turn from here.
* **The sign-in recipes are one table, and they are not exercised against the real CLIs here.** SDC
  drives `claude` / `codex` / `gemini`'s own login (`auth::cli_login::RECIPES`) and proves the
  mechanism against a stand-in CLI that behaves the same way (prints a URL, waits for a line,
  announces success). None of the three is installed on the machine this was built on, so a CLI that
  changes its command or its success line needs that table corrected - which is why the UI shows the
  CLI's raw output instead of only "failed". SDC never sees the credential either way: the CLI writes
  it to its own store.
* **A model list is live whenever it can be, and the row says which of the three it is.** `models.list` asks
  each provider's own endpoint, caches the answer with the time it arrived, and falls back to
  `protocol/models.json`; the three sources are `live`, `cache` and `bundled`. A remote provider's list is
  fetched over TLS (the cache in a working install carries `fetchedAt` timestamps from real fetches); a build
  with no network, or a provider that answers with an error, falls back and says so.
* **The native API's TLS client exists, and this README claimed it did not.** `native_api` builds an Anthropic
  `messages` request or an OpenAI `chat/completions` request, parses both SSE dialects, and has streamed an
  `https://` endpoint over `ureq` (rustls + the webpki roots) since **0.6.1** - `post_https` in
  `engines/native_api.rs`. The sentence that stood here ("a remote `https://` endpoint answers *a TLS client is
  not linked in this build*… the next crate to add (`rustls` + `hyper`)") was two releases out of date, and
  0.7.11 **measured** it instead of re-reading it: a key nobody wants, sent to the provider's own model list,
  came back with the provider's sentence and `verified: true` (`_verify/live-provider-test.mjs`). What is still
  narrow is the attention this build has had on the *streaming* remote path - the transport under it is the same
  agent, but a turn against a paid endpoint has not been exercised from here - and the `http://` loopback path
  that LM Studio, vLLM and llama.cpp speak, which is the one a test can drive without a network.
* **`session.fork` no longer answers `unknown method`.** Since 0.7.8 the daemon copies a chat's turns into a
  new chat (its own rows, `atTurn` inclusive, replayed into the log so the fork's transcript *is* the
  conversation) and the sidebar has a **Fork** button on every row plus a `Fork this chat` palette row.
* **`cli.recipes` is read before a sign-in, not after a failure.** The Connect dialog shows the provider's
  recipe first (`` `claude` is installed `` / `` is not installed ``, the daemon's own install words, a Copy
  button and Check again) and no longer starts a login for a program this machine does not have.
* **The provider OAuth token exchange.** `provider.oauth.open` returns a real URL and a real `state`;
  `provider.oauth.callback` records the state and says the exchange lands with the OAuth step. No
  token is invented, because a fake token would fail later in a stranger place.
* **Checkpoint screenshots.** `checkpoints::screenshot` owns the path and the contract; the pixels
  come from the app's WebView (`checkpoints::screenshot::record`), so a thumbnail appears when the
  preview writes one. The rewind does not depend on it: files are restored from the shadow
  repository, which is real today.
* **The OS keychain is used where the platform has one, and the fallback is protected where it is not.**
  Since 0.7.10 the packaged daemon is built with `keychain` on Windows (DPAPI) and macOS (Keychain);
  `backend()` is a *runtime* answer, because `keyring` v3 compiles with no backend at all and even a
  compiled-in store can be unreachable - a locked Keychain or a service account falls back to the file, and the
  reported store and the used store cannot disagree. Linux keeps the file fallback on purpose: the Secret
  Service needs `dbus` headers and a running session bus, and a daemon that cannot start on a headless box is
  worse than one that says which store it used. On Windows that file now gets an ACL
  (`icacls /inheritance:r /grant:r <account>:F`) instead of inheriting `%APPDATA%` - until 0.7.10 a key was
  readable by every account in `Users`. `host.status` reports both: `keychain` (`os`/`file`) and
  `keyProtection` (`os`/`acl`/`mode`). The fallback's directory is `0700` and the file `0600`, which 0.7.12 had
  to be told by CI: `0600` on a directory clears the execute bit, and a directory you cannot search is a
  directory no key can be written into.
* **`provider.test` really contacts the provider, and says so in `verified`.** Since 0.6.1 a key is used for
  one read-only call to the provider's own model list over TLS, and both outcomes are first-class: a key the
  provider accepts answers with the models it knows and `verified: true`, and a key it rejects answers with the
  provider's own sentence (`API key is invalid. (401)`) and `verified: true` as well, because the question *was*
  asked. `verified: false` is what "we could not ask" means - no key stored, or the endpoint unreachable - and
  0.7.11 measured the first case live rather than trusting the note that used to be here
  (`_verify/live-provider-test.mjs`). A local Ollama daemon is probed the same way, over `http://`.
* **Installers for macOS and Linux, and a signed build.** CI publishes all of them (`*.dmg` ×2,
  `*.AppImage`, `*.deb`, `*.msi`, `*_x64-setup.exe`); on Windows the **installed** app was verified end
  to end in 0.4.4 — silent install, the window renders, the daemon it started is 0.4.4, no console
  window, and both the app and its daemon are gone after quitting. macOS and Linux artifacts are built
  and published but have not been opened on their own hosts from here, and signing needs certificates
  this repository does not hold.
* **An axe-core pass runs in CI; the screen-reader sweep is still a person's job.** Since 0.7.10
  `app/scripts/a11y-bundle.mjs` audits the built window over WCAG 2.0/2.1 A + AA with `serious`/`critical`
  failures failing the build, and it reports **0 violations** on the screen the app opens on. What no automated
  pass covers is the rest of the window - the palette, the Permission modal, the Time Machine tab - and what a
  screen reader actually *says*; that sweep is still on the list below. One thing 0.7.12 learned about the
  audit itself: its verdict depends on the **state** the app opens with. 0.7.10's green run was green because
  that install had no chats, and the `nested-interactive` violation it missed needs a host with rows under it;
  the fix was verified against a populated window.
* **`protocol/` is not an npm package yet, and it no longer drifts silently.** `protocol/check.mjs` (0.7.10)
  compares the schema's method names, shapes and event types with `protocol/types.ts` **and with the daemon's
  dispatch table**, and it runs in CI - which is what the missing generator was for. Turning the folder into a
  workspace package is still open, and it is now a packaging question rather than a safety one: the check is
  what makes the two unable to disagree, and a package would only change who imports what.
* **A second `sdcd` on a host over an `ssh -L` tunnel - an alternative, not a missing layer.** 0.7.13's remote
  layer is **exec-based**: every operation against a host is an `ssh` command (`docs/REMOTE.md`), including the
  engines (`cd <folder> && sh -c 'mkdir -p …; echo $$ > <pid>; exec setsid … <cli> …'`, with a **process group**
  so a cancel kills the tree and not one process), the checkpoints (a shadow git repository at
  `$HOME/.sdc/git/<hash>` **on that host**), long-running processes (`pty.open { line, hostId }` — an `ssh`
  whose remote process writes the same kind of pid file, so output, stdin and cancellation all work), and
  `host.doctor`, which answers about the host itself. The tunnel design - a Linux `sdcd` copied to each host,
  reached over a forwarded port - would buy a far-side process to watch files, hold a PTY and cache state; of
  those, the PTY is now done over exec, file watching is a feature this window does not have *locally* either,
  and the cost is an artifact per architecture, a listening port and a token to provision, and a second event
  log to reconcile with this one. SDCP's `ws` transport (`VITE_SDCP_URL`) is still there for it, so it is a
  design decision with a stated condition to revisit rather than work left undone. See `sdc/docs/REMOTE.md` §5
  for the trade-off as a table.
* **The engines' CLIs still have to be installed on the host.** A turn there runs `claude`/`codex`/`gemini` on
  that machine, so a host without them answers `not installed` in its doctor row and a turn fails in the CLI's
  own words. SDC does not install anything on somebody else's server - but the row's `Install` button opens the
  **Terminal on that host**, so the person runs the installer themselves, in that folder, with the checkpoint,
  the deny list and the tool-call record every other command gets.
* **The editor is an editor now, and still not an IDE.** Since 0.8.0 a file opens in a CodeMirror 6 tab
  (highlighting, search, undo, Ctrl+S with a checkpoint first), the tree creates, renames, deletes and
  searches through the daemon, and answers render Markdown. What stays absent on purpose: language services
  (completion, go-to-definition), a debugger and extensions - SDC is not an IDE (spec §2.2) - and a marker
  for which line a turn touched.

## Next step

1. **A paid live turn in the Anthropic dialect** (thinking with its signature, tool results), to go with the
   recorded stream the tests replay. The VPS path itself was driven end to end against the owner's own
   server in 0.9.0: sign-in with a password and a verification code, the tree, remote file ops, and a
   Claude Code agent turn that edited a file there.
2. **A key for one of the six new providers** (Google, xAI, Moonshot, Mistral, Qwen, Z.ai), to exercise
   their live list and a chat turn from here; their blocks follow the same OpenAI shape the measured
   providers use, but none of the six has been contacted from this machine.
3. **A terminal emulator** - `xterm.js` in the window and `-tt` on a host - for full-screen programs; the
   Terminal tab runs commands and long-running processes today, but it is not a PTY.
4. The provider OAuth token exchange, which needs a client registration with each provider.
5. A screen-reader pass over the palette, the Permission modal and the Time Machine tab.
6. Code signing for the installers, which needs certificates this repository does not hold.
