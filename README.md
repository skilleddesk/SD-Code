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
| [`CHANGELOG.md`](CHANGELOG.md) | What changed, release by release — `v0.4.1` is the daemon step |
| [`sdc/docs/MASTER_SPEC.md`](sdc/docs/MASTER_SPEC.md) | The master build specification v3.0 |
| [`sdc/design/ui-prototype.html`](sdc/design/ui-prototype.html) | The working UI prototype — the UI source of truth (spec §7) |
| [`LICENSE`](LICENSE) | Proprietary licence, all rights reserved |
| [`SECURITY.md`](SECURITY.md) | Vulnerability disclosure policy — **report privately, never in a public issue** |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Contribution terms, conventions and the secret rules |
| [`.env.example`](.env.example) | Environment template: variable names and comments, never values |

## Install on Windows (v0.4.1)

Two ways, from the `release/` folder of a checkout that has been built (`pnpm tauri:build`):

| File | What you do |
| --- | --- |
| `SDC_0.4.1_x64-setup.exe` | **Double-click → Next → Install.** Adds SDC to the Start Menu; uninstall from *Apps & features*. |
| `SDC-0.4.1-portable.zip` | Unzip anywhere and double-click `sdc.exe`. Keep `sdcd.exe` in the same folder — `sdc.exe` starts it. |

Either way the window opens and **starts the daemon itself**: the first thing the app does is call
`host.status`, and the Tauri bridge answers a refused connection by spawning `sdcd` (next to the app
binary) and waiting for `127.0.0.1:7811`. Your data lives in `%APPDATA%\sdc\` — `sdc.db` holds the
session rows and the append-only event log, and `keys\` holds provider secrets until the `keychain`
feature is on.

The installers are ~4 MB and unsigned, so Windows SmartScreen will show *"Windows protected your
PC"* the first time: choose **More info → Run anyway**. Code signing is on the next-step list in
`sdc/README.md`.

To build them yourself on a Windows machine with the toolchain from `sdc/SETUP.md`:

```bash
cd sdc
pnpm install
pnpm tauri:build        # builds sdcd, bundles it as a sidecar, produces .msi + .exe
```

Output lands in `sdc/app/src-tauri/target/release/bundle/{msi,nsis}/`.

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
WebView owns the pixels; the daemon owns the path), the macOS and Linux installers, and an axe-core
pass. `sdc/README.md` tracks exactly what exists, what is deliberately absent, and what comes next.

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
