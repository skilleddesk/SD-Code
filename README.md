<div align="center">

<img src="sdc/app/src-tauri/icons/128x128@2x.png" alt="SDC — Skilleddesk Code" width="112" height="112" />

<h1>SDC — Skilleddesk Code</h1>

<p><strong>A local-first coding-agent workbench.</strong><br />
A Tauri 2 desktop app, a host daemon (<code>sdcd</code>) and an append-only event log.</p>

<p>
  <sub><strong>PROPRIETARY SOFTWARE — ALL RIGHTS RESERVED. NOT OPEN SOURCE.</strong></sub><br />
  <sub>Published for reading, review and security audit only. No licence to copy, modify<br />
  or redistribute is granted. See <a href="LICENSE">LICENSE</a>.</sub>
</p>

</div>

---

## Where the code lives

The product is in [`sdc/`](sdc/README.md). This repository root holds only the
project's own metadata — licence, security policy, contribution terms and CI.

| Path | What it is |
| --- | --- |
| [`sdc/`](sdc/README.md) | The monorepo: `app/` (Tauri 2 + React 18 + TypeScript + Tailwind), `sdcd/` (host daemon, Rust), `protocol/`, `design/`, `docs/` |
| [`sdc/README.md`](sdc/README.md) | Layout, commands and the decisions behind them — start here |
| [`sdc/SETUP.md`](sdc/SETUP.md) | Fresh-machine toolchain guide (Node 20 LTS, pnpm 10, Rust, Tauri CLI, platform dependencies) |
| [`sdc/docs/RELEASE.md`](sdc/docs/RELEASE.md) | The release checklist: the gates, the packaged-app check, and what to do when a job fails |
| [`CHANGELOG.md`](CHANGELOG.md) | What changed, release by release — `v0.4.1` is the daemon step |
| [`sdc/docs/MASTER_SPEC.md`](sdc/docs/MASTER_SPEC.md) | The master build specification v3.0 |
| [`sdc/design/ui-prototype.html`](sdc/design/ui-prototype.html) | The working UI prototype — the UI source of truth (spec §7) |
| [`LICENSE`](LICENSE) | Proprietary licence, all rights reserved |
| [`SECURITY.md`](SECURITY.md) | Vulnerability disclosure policy — **report privately, never in a public issue** |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Contribution terms, conventions and the secret rules |
| [`.env.example`](.env.example) | Environment template: variable names and comments, never values |

## Install (Windows / macOS / Linux)

**The easy way:** open [Releases](https://github.com/skilleddesk/SD-Code/releases) and download the file
for your machine. Every release is built on its own runner — Windows, macOS (Intel and Apple Silicon)
and Linux — by [`.github/workflows/release.yml`](.github/workflows/release.yml).

| Your machine | Download | What you do |
| --- | --- | --- |
| Windows | `SDC_<version>_x64-setup.exe` | Double-click → Next → Install. Start Menu → **SDC**. Uninstall from *Apps & features*. |
| Windows (managed) | `SDC_<version>_x64_en-US.msi` | The same, for a fleet or a silent install. |
| macOS, Apple Silicon | `SDC_<version>_aarch64.dmg` | Open the `.dmg`, drag **SDC** to Applications, then **right-click → Open** the first time. |
| macOS, Intel | `SDC_<version>_x64.dmg` | Same. |
| Linux | `SDC_<version>_amd64.AppImage` | `chmod +x SDC_*.AppImage` then run it — no install, no root. |
| Linux (Debian/Ubuntu) | `SDC_<version>_amd64.deb` | `sudo apt install ./SDC_*.deb`. |

Two things to expect, and neither is a bug: the builds are **unsigned**, so Windows SmartScreen says
*"Windows protected your PC"* (**More info → Run anyway**) and macOS Gatekeeper asks once
(right-click → Open). Only the files that launched the first time carry the mark; that is what signing
would remove, and it is on the next-step list.

**The manual way**, on a machine with the toolchain from `sdc/SETUP.md`:

```bash
cd sdc
pnpm install
pnpm tauri:build        # builds sdcd, bundles it as a sidecar, produces .msi + .exe (or .dmg / .AppImage / .deb)
```

Output lands in `sdc/app/src-tauri/target/release/bundle/`. To build the installers for a tag from
GitHub instead, either push a `v*` tag or run the **release** workflow from the Actions tab.

The daemon is bundled into every installer: the app starts `sdcd` itself on first launch, and your data
lives in `%APPDATA%\sdc` (Windows) or `~/.local/share/sdc` (Linux/macOS) — `sdc.db` for the sessions
and the append-only event log.

## How to work with it

1. **Open it.** The window appears and the status bar's right-hand dot turns green: that is `sdcd`
   answering `host.status` with *this* machine's real state.
2. **Check the environment.** Settings → Environment (**F1** shows every shortcut). The doctor's ten
   rows are live: Node, Claude Code CLI, Codex, Gemini, Ollama, ripgrep, port 3000, disk, git, ssh.
   Anything missing has an **Install** button and a sentence saying what it is for.
3. **Connect an engine.** Provider Hub → pick one of the nine.
   *A subscription* (Claude Code, Codex, Gemini) opens **Connect → Sign in**: SDC starts the CLI's own
   login, shows the link with a copy button, and takes the code you paste back. The credential is
   written by the CLI, never by SDC, and the CLI's own output is shown underneath so a stale recipe is
   visible. The daemon reports whether the CLI is installed at all (`cli.recipes`), so a missing
   `claude` says "install it first" instead of failing mysteriously.
   *An API provider* opens **Connect → API key and model**: the key goes to the keychain, then
   **Load models** / **Refresh** lists what the provider offers. Every row says where it came from —
   `live` (the provider answered just now), `cached` (its last answer) or `bundled` (shipped with this
   build) — and **Use** records your choice. A model the provider has that this build never heard of
   still appears, so nothing has to be added to the code when a new version ships.
   Ollama is the local engine and works end to end today.
4. **Ask for something.** Type in the prompt area and press **Enter**. The turn streams: thinking,
   tool calls, the answer. A mutating step writes a checkpoint *before* it runs, so
   **Time Machine → Rewind** (or `Ctrl+Z` in that tab) puts the files and the conversation back.
5. **Approve or refuse.** A dangerous action raises the **Permission** dialog with the file, the risk
   and three buttons: *Deny*, *Allow once*, *Always allow*. Nothing mutating happens without that step.
6. **Run a command yourself.** `shell.run` is the daemon's execute step — one command, its output
   captured, its exit code reported, and a non-zero exit translated into a sentence. In the desktop
   app an engine uses it; from the protocol it is one call:
   `{ "method": "shell.run", "params": { "command": "pnpm", "args": ["test"] } }`.
7. **Duel.** Right panel → **Duel** runs one prompt on two engines and lets you keep a winner; the
   loser is archived, never deleted.
8. **Switch mid-turn.** **Session Bridge** hands the conversation to another engine without losing it —
   the history is replayed into the new engine, not the tokens.
9. **Where things are.** `%APPDATA%\sdc\sdc.db` (sessions, turns, the event log), `%APPDATA%\sdc\keys\`
   (provider secrets until the OS keychain feature is on), `<data>/git/` (the shadow repositories a
   rewind restores from).

Working on the code itself: `sdc/README.md` has the layout, the commands and the reasoning;
`CONTRIBUTING.md` has the gate to run before a commit.

## Status

Early, but no longer a shell. The desktop app is complete — topbar, sidebar, tabs, turn stream,
prompt area, right panel, status bar, command palette, Provider Hub, Settings, the Time Machine, Duel
mode and the F1 keymap reference, all folded from one append-only event log. The host daemon `sdcd`
implements the whole plan: SDCP over newline-delimited JSON, five engine adapters (Claude Code, Codex,
Gemini, a native-API engine and Ollama), the file guard, a shadow git repository, processes, the OS
keychain, the ten environment checks, the provider backend, checkpoints and rewind, and the Session
Bridge. The Tauri bridge (`sdc/app/src-tauri/src/sdcp.rs`) carries SDCP into the window and starts the
daemon if it is not running.

What is open is named rather than hidden: a TLS client for the *remote* native-API endpoints (a local
`http://` one streams today), the provider OAuth token exchange, checkpoint screenshots (the app's
WebView owns the pixels; the daemon owns the path), signing and notarisation, and an axe-core pass.
`sdc/README.md` tracks exactly what exists, what is deliberately absent, and what comes next.

One thing 0.5.0 settled is worth stating here, because it is the question a first launch raises: the
window **contains no demo content**. It used to - three hosts, six chats, nine providers, twelve models
and a fabricated turn stream - which meant a fresh install looked connected and busy while the daemon
behind it answered nothing. What it shows now is the event log: `host.status`, `session.list`,
`provider.list` and the turn events. If it is empty, it is empty because nothing has happened yet, and
the empty states say so.

## Licence

SDC is the proprietary property of SkilledDesk. All rights reserved.

Being able to read this repository does not make it open source and does not
grant you a licence: you may not copy, modify, redistribute, sublicense, sell,
reverse engineer or use it — in whole or in part, including as training or
evaluation data for a machine-learning model — without prior written permission.
See [`LICENSE`](LICENSE) for the full terms.

For the avoidance of doubt: no MIT, Apache, BSD, GPL, MPL or other open-source
licence applies to this project. Third-party dependencies named in
`sdc/app/package.json` and the `Cargo.toml` files remain under their own licences
and are unaffected by the above.

Licensing and commercial enquiries: `<OWNER-CONTACT-EMAIL>`.

## Security

Please do not open a public issue for a vulnerability. Use GitHub's private
reporting (repository → **Security** → **Report a vulnerability**) or the address
in [`SECURITY.md`](SECURITY.md). There is no bug bounty. That file also lists
what is in scope — credential handling, unconsented egress, the permission
broker, undo integrity — and the fact that secrets are scanned for on every push.
