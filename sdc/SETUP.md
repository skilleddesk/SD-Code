# SDC (Skilleddesk Code) — Local Development Setup

Step-by-step setup guide for a **fresh machine** (Windows, Linux or macOS) that will
build the SDC Tauri 2 desktop application.

> **Scope of this step.** The deliverables are this document, the two dotfiles next to it,
> and the spec/design reference artefacts listed below. No application is scaffolded here:
> there is no `package.json`, no `src-tauri/` and no `tauri.conf.json` yet, and no runtime
> dependency is installed. Application scaffolding is a later step.

> **Update 2026-09-19 — the scaffold now exists (STEP 1).** `sdc/app/` (Tauri 2 + React 18 +
> TypeScript + Tailwind), `sdc/sdcd/` (host daemon) and the pnpm workspace root are in place,
> with a "Hello SDC" window and no feature code. `README.md` documents the layout, the
> commands, and the decisions (including why the spec's `make gen` becomes the pnpm script
> `tokens:gen` instead of a Makefile). This guide remains the toolchain reference; §10 and §11
> below record what the scaffold changed on this machine and what is still missing.

| Item | Value |
| --- | --- |
| App framework | Tauri 2 |
| Targets | Windows 10/11 (x64), Linux (x64), macOS 10.15+ (x64 / Apple silicon) |
| Node.js | **20 LTS** |
| Frontend package manager | **pnpm** |
| Rust | **stable** toolchain via `rustup` |
| Tauri CLI | **`tauri-cli ^2`** (invoked as `cargo tauri`) |
| Master build spec | `sdc/docs/MASTER_SPEC.md` |
| UI prototype (UI source of truth) | `sdc/design/ui-prototype.html` |
| Design tokens | `sdc/design/tokens.json` |

Files delivered in this step (all inside the `sdc/` project directory):

| File | Purpose |
| --- | --- |
| `sdc/SETUP.md` | This guide |
| `sdc/.editorconfig` | 2-space indent, LF endings, UTF-8, trim trailing whitespace |
| `sdc/.gitignore` | `node_modules/`, `dist/`, `target/`, `.env`, `*.tar.age`, `.DS_Store` |
| `sdc/docs/MASTER_SPEC.md` | SDC master build specification v3.0 — §7–§9 fully define the UI |
| `sdc/design/ui-prototype.html` | Working single-file UI prototype; per spec §7 it wins over the doc |
| `sdc/design/tokens.json` | Design tokens taken from the prototype CSS and spec §8.1–§8.5 |

Shell used for the commands below:

| OS | Shell |
| --- | --- |
| Windows | PowerShell (every Windows command in this guide is PowerShell-compatible) |
| Linux | `bash` / `zsh` |
| macOS | `zsh` (default) / `bash` |

---

## 0. Install order

1. [Git](#1-git)
2. [Node.js 20 LTS](#2-nodejs-20-lts)
3. [pnpm](#3-pnpm)
4. [Rust stable toolchain (rustup)](#4-rust-stable-toolchain-rustup)
5. [Tauri 2 CLI](#5-tauri-2-cli-cargo-install-tauri-cli)
6. [Platform dependencies](#6-platform-dependencies)
7. [Editor configuration](#7-editor-configuration)
8. [Verify the whole toolchain](#8-verification-checklist)

**Open a new terminal after steps 2, 4 and 5** (they change `PATH`). After installing the
Windows MSVC build tools, a sign-out or reboot is occasionally required.

### The stack this environment must support (locked by the master spec)

`docs/MASTER_SPEC.md` §4.1 locks the stack; the toolchain installed by this guide is exactly
what that stack needs.

| Layer | Locked choice | Provided by |
| --- | --- | --- |
| App shell | Tauri 2 | §4 (Rust stable) + §5 (`tauri-cli ^2`) |
| Frontend | React 18 + TypeScript + Tailwind CSS | §2 (Node.js 20 LTS) + §3 (pnpm) |
| Icons | Lucide React — 16 px default, `stroke-width: 1.5` | installed as a dependency at scaffold time |
| Fonts | Inter (UI) + JetBrains Mono (code, paths, diffs) | installed at scaffold time |
| Design tokens | CSS variables → `design/tokens.json` → `make gen` (tokens.css + Tailwind config) | `sdc/design/tokens.json` (already in the repo) |
| UI strings | every string in one file, `app/src/strings.ts` (spec §2.7) | scaffold time |
| Protocol / data | SDCP over the `sdcd` daemon; SQLite with FTS5 | later phase (needs the v2.0 spec, see §11) |

Two spec rules that bind every later code change:

1. **No component may hardcode a colour** — semantic token names only (spec §8.1).
2. **Where the doc and the prototype disagree, the prototype wins** (spec §7).

Source-tree paths in the spec start with `app/` (e.g.
`app/src/panels/sessions/SidebarHostGroup.tsx`), so the scaffold will live in `sdc/app/`
with the Rust crate in `sdc/app/src-tauri/`.

---

## 1. Git

**Windows** — `winget` ships with Windows 10/11:

```powershell
winget install --id Git.Git --exact --accept-source-agreements --accept-package-agreements
```

**macOS** — `xcode-select --install` (see §6.3) already provides `git`; for a newer build
use Homebrew: `brew install git`.

**Linux (Debian/Ubuntu)**:

```sh
sudo apt update
sudo apt install git
```

Confirm and configure (once per machine):

```sh
git --version          # git version 2.x.y

git config --global user.name  "Your Name"
git config --global user.email "you@example.com"
# .editorconfig pins LF in the working tree, so disable Git's CRLF rewriting:
git config --global core.autocrlf false
git config --global init.defaultBranch main
```

**If `sdc/` is not a Git repository yet**, initialise it once from the project directory
and commit the three files from this step:

```sh
cd sdc
git init
```

Recommended editor/merge tool: VS Code (`git config --global core.editor "code --wait"`).

---

## 2. Node.js 20 LTS

SDC targets **Node.js 20 LTS**. Pick **one** route — the version-manager route is
recommended because it lets Node 20 coexist with any newer release already on the machine.

### Route A — version manager (recommended)

**Windows** — nvm-windows manages Node versions natively on Windows:

```powershell
winget install --id CoreyButler.NVMforWindows --exact --accept-source-agreements --accept-package-agreements

# in a NEW terminal:
nvm install 20
nvm use 20
nvm list                # 20.x.y should be present
```

**Linux / macOS** — nvm for POSIX shells, using the install script linked from
<https://github.com/nvm-sh/nvm#install--update-script>:

```sh
nvm install 20
nvm use 20
nvm alias default 20    # make Node 20 the default in new shells
```

`fnm` is a lighter alternative:

```powershell
winget install --id Schniz.fnm --exact --accept-source-agreements --accept-package-agreements   # Windows
```

```sh
fnm install 20          # Linux / macOS (run `fnm env` setup once for your shell, see fnm docs)
fnm default 20
```

### Route B — official installer (Windows / macOS)

Download the **Node.js 20 LTS** installer for your OS from
<https://nodejs.org/dist/latest-v20.x/> and run it:

| OS | File |
| --- | --- |
| Windows x64 | `node-v20.20.2-x64.msi` |
| macOS (universal) | `node-v20.20.2.pkg` |
| Linux x64 (if you prefer not to use a package manager) | `node-v20.20.2-linux-x64.tar.xz` |

> `latest-v20.x` is the newest 20.x patch release — `v20.20.2` at the time of writing.
> Any 20.x patch is fine for local development; the point is the **20.x** major line.
> The same directory publishes `SHASUMS256.txt` if you want to verify the download.

### Route C — Homebrew (macOS, optional)

```sh
brew install node@20
```

Upstream marks this formula deprecated and it is keg-only, so `node` may need
`brew link --overwrite --force node@20`. Prefer Route A unless you already use Homebrew.

### Verify Node

```sh
node -v                 # v20.x.y
npm -v
```

Open a new terminal before continuing.

---

## 3. pnpm

`corepack` is bundled with Node 20 and can enable pnpm without a global install:

```sh
corepack enable pnpm
pnpm -v
```

Alternatives:

```sh
npm install -g pnpm     # any OS, using the npm installed above
```

```powershell
winget install --id pnpm.pnpm --exact --accept-source-agreements --accept-package-agreements   # Windows
```

When the frontend workspace is created in the later scaffold step, the exact pnpm version
will be pinned in the repository (`packageManager` field) so every machine resolves the
same one.

---

## 4. Rust stable toolchain (rustup)

`rustup` is the supported way to install the Rust toolchain Tauri compiles against.

**Linux and macOS** — the official rustup command:

```sh
curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh
```

Follow the on-screen prompts and accept the default (`stable`) toolchain. The installer
appends `$HOME/.cargo/bin` to your shell profile; open a new terminal afterwards.

**Windows** — install rustup with winget:

```powershell
winget install --id Rustlang.Rustup --exact --accept-source-agreements --accept-package-agreements
```

Alternative: download and run `rustup-init.exe` from
<https://www.rust-lang.org/tools/install> (direct link: <https://win.rustup.rs/x86_64>).
The binaries land in `%USERPROFILE%\.cargo\bin`, which the installer adds to `PATH`.

> **Windows only — MSVC host triple.** Tauri needs the MSVC toolchain, so the *default
> host triple* must be `x86_64-pc-windows-msvc` (or `i686-pc-windows-msvc` /
> `aarch64-pc-windows-msvc` for other architectures). Choose it in the rustup-init
> dialog, and if you are unsure, force it afterwards:

```powershell
rustup default stable-msvc
```

**Any OS, if Rust was already installed** — make sure stable is current:

```sh
rustup update stable
rustup show             # lists installed toolchains + the active default
```

The Tauri prerequisites page adds: *"Be sure to restart your Terminal (and in some cases
your system) for the changes to take effect."*

---

## 5. Tauri 2 CLI (`cargo install tauri-cli`)

Install the Tauri 2 CLI globally with Cargo — identical on Windows, Linux and macOS:

```sh
cargo install tauri-cli --version "^2"
```

Notes:

- The `"^2"` quoting above works as written in PowerShell, `cmd`, `bash` and `zsh`;
  no administrator rights are required.
- The CLI is compiled from source on first install, so the command takes several minutes.
- The binary is placed in the Cargo bin directory —
  `%USERPROFILE%\.cargo\bin\tauri.exe` on Windows, `$HOME/.cargo/bin/tauri` on
  Linux/macOS — and is invoked through Cargo as **`cargo tauri`**.
- Verify: `cargo tauri -V` → `tauri-cli 2.x.y`.
- Re-run the same command to upgrade; `cargo uninstall tauri-cli` removes it.

Project-local CLI wiring (for example invoking the CLI through the frontend package
manager, `pnpm tauri …`) belongs to the later scaffold step and is intentionally **not**
part of this setup.

---

## 6. Platform dependencies

### 6.1 Windows

Tauri on Windows needs two OS-level components: the **Microsoft C++ Build Tools** (the
MSVC linker Rust uses) and the **Microsoft Edge WebView2** runtime (what renders the UI).

**Microsoft C++ Build Tools**

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --accept-source-agreements --accept-package-agreements
```

Then, in the Visual Studio Installer, tick the **"Desktop development with C++"**
workload (it brings the MSVC compiler/linker and the Windows SDK) and let it finish.
GUI alternative: download the installer from
<https://visualstudio.microsoft.com/visual-cpp-build-tools/> and check the same option.

> `cl.exe` will **not** appear on your `PATH` — that is expected. Cargo locates the MSVC
> linker from the registry, so everything works as long as the C++ workload got installed.

**WebView2**

WebView2 is preinstalled on Windows 10 (1803 and later) and Windows 11, so most machines
can skip this. Check whether it is present:

```powershell
winget list --id Microsoft.EdgeWebView2Runtime --exact --accept-source-agreements
```

If it is missing, install the Evergreen runtime:

```powershell
winget install --id Microsoft.EdgeWebView2Runtime --exact --accept-source-agreements --accept-package-agreements
```

or download the **Evergreen Bootstrapper** from
<https://developer.microsoft.com/en-us/microsoft-edge/webview2/#download-section>.

**Smoke test that the MSVC toolchain can actually compile and link** — this proves the
C++ workload is complete. It runs entirely in `%TEMP%` and does not touch the `sdc/` tree:

```powershell
Set-Content -Path "$env:TEMP\sdc-link-test.rs" -Value 'fn main() { println!("link ok"); }'
rustc "$env:TEMP\sdc-link-test.rs" -o "$env:TEMP\sdc-link-test.exe"
& "$env:TEMP\sdc-link-test.exe"      # prints: link ok
Remove-Item "$env:TEMP\sdc-link-test.*"
```

If this fails with `link.exe not found` (or `LINK : fatal error`), the "Desktop
development with C++" workload is missing — re-run the Build Tools installer and add it.
Separately, building **MSI** installers later requires the optional **VBSCRIPT** Windows
feature (*Settings → Apps → Optional features → More Windows features*), which is enabled
by default on most systems.

### 6.2 Linux

Tauri 2 needs the WebKitGTK **4.1** development package (4.0 is not enough), GTK 3
development headers, the Ayatana AppIndicator 3 library (for tray icons) and librsvg.

**Debian / Ubuntu — the dependencies required by SDC:**

```sh
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

Tauri upstream also requires a C toolchain plus a few helpers; this is the upstream
Debian set and is a **superset** of the list above (either command is fine — the second is
the safer one):

```sh
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

**Fedora** (upstream set):

```sh
sudo dnf check-update
sudo dnf install webkit2gtk4.1-devel \
  openssl-devel \
  curl \
  wget \
  file \
  libappindicator-gtk3-devel \
  librsvg2-devel \
  libxdo-devel
sudo dnf group install "c-development"
```

**Arch** (upstream set):

```sh
sudo pacman -Syu
sudo pacman -S --needed \
  webkit2gtk-4.1 \
  base-devel \
  curl \
  wget \
  file \
  openssl \
  appmenu-gtk-module \
  libappindicator-gtk3 \
  librsvg \
  xdotool
```

For every other distribution, see
<https://v2.tauri.app/start/prerequisites/#linux> and
<https://github.com/tauri-apps/awesome-tauri#guides>.

**Verify** the libraries are visible to `pkg-config` (this is how Cargo finds them):

```sh
pkg-config --modversion webkit2gtk-4.1 gtk+-3.0 librsvg-2.0
pkg-config --exists ayatana-appindicator3-0.1 && echo "appindicator OK"
```

> Older distributions (for example Ubuntu 20.04) only ship WebKitGTK **4.0**
> (`libwebkit2gtk-4.0-dev`) and cannot build Tauri 2 — use a release that provides 4.1
> (Ubuntu 22.04 or newer).

### 6.3 macOS

Tauri supports macOS 10.15 (Catalina) and later. For **desktop** development the Xcode
Command Line Tools are sufficient (full Xcode is only needed for iOS targets):

```sh
xcode-select --install
```

Accept the licence if you are prompted:

```sh
sudo xcodebuild -license accept
```

**Verify:**

```sh
xcode-select -p          # /Library/Developer/CommandLineTools  (or a path inside Xcode.app)
clang --version          # Apple clang version ...
```

If you installed full Xcode instead, launch it once so it can finish its first-run setup.

---

## 7. Editor configuration

`sdc/.editorconfig` is committed with the project and is the single source of truth for
basic formatting. The complete file:

```ini
# SDC (Skilleddesk Code) — EditorConfig
# https://editorconfig.org
# Tauri 2 desktop app: Rust (src-tauri) + web frontend.

# This is the top-most EditorConfig file: stop looking for parent configs.
root = true

# Applies to every file in this repository.
[*]
charset = utf-8
end_of_line = lf
indent_style = space
indent_size = 2
trim_trailing_whitespace = true
insert_final_newline = true
```

Editor support:

- **VS Code** — install the official **"EditorConfig for VS Code"** extension
  (marketplace id `editorconfig.editorconfig`). It supports exactly the properties used
  above (`indent_style`, `indent_size`, `end_of_line`, `insert_final_newline`,
  `trim_trailing_whitespace`, `charset`); `end_of_line`, `insert_final_newline` and
  `trim_trailing_whitespace` are applied **on save**. Rust support for the future
  `src-tauri` crate comes from the `rust-analyzer` extension.
- **JetBrains IDEs** — built-in support: enable *Settings → Editor → Code Style →
  Enable EditorConfig support*.
- **Vim / Neovim / Emacs** — use the respective `editorconfig` plugin.

Two notes about `trim_trailing_whitespace = true`: it strips the trailing spaces that
Markdown otherwise treats as hard line breaks (use a blank line instead), and in VS Code
the extension *cannot* disable trailing-whitespace trimming if the user/workspace setting
`files.trimTrailingWhitespace` is set to `true` — leave that VS Code setting alone.

`sdc/.gitignore` keeps generated and machine-specific artefacts out of Git:

```gitignore
# SDC (Skilleddesk Code) — Git ignore rules

# Node / pnpm dependencies
node_modules/

# Frontend build output
dist/

# Rust / Tauri build output
target/

# Local environment files (never commit secrets)
.env

# Encrypted backup artifacts
*.tar.age

# macOS metadata
.DS_Store
```

`dist/` and `target/` do not exist yet; ignoring them now means the build output of the
first `cargo tauri build` can never be committed by accident. Only the exact `.env` name
is ignored by that pattern.

---

## 8. Verification checklist

Run everything from a **new** terminal. The five baseline checks are the acceptance
criteria for this step and are identical on every OS:

| # | Command | Expected result |
| --- | --- | --- |
| 1 | `node -v` | `v20.x.y` |
| 2 | `pnpm -v` | a version number (not "command not found") |
| 3 | `rustc -V` | `rustc 1.xx.y (… 20xx-xx-xx)` |
| 4 | `cargo -V` | `cargo 1.xx.y (…)` |
| 5 | `cargo tauri -V` | `tauri-cli 2.x.y` |

All five in one line (PowerShell, bash and zsh compatible):

```sh
node -v && pnpm -v && rustc -V && cargo -V && cargo tauri -V
```

Platform-specific additions:

| OS | Command | Expected result |
| --- | --- | --- |
| Windows | `rustup show` | default host triple ends in `-msvc`; active toolchain `stable-x86_64-pc-windows-msvc` |
| Windows | `winget list --id Microsoft.EdgeWebView2Runtime --exact` | webview2 runtime listed |
| Windows | the `rustc` compile/link smoke test in §6.1 | prints `link ok` |
| Linux | `pkg-config --modversion webkit2gtk-4.1 gtk+-3.0 librsvg-2.0` | three version numbers, no error |
| Linux | `pkg-config --exists ayatana-appindicator3-0.1 && echo OK` | `OK` |
| macOS | `xcode-select -p` | a CommandLineTools or Xcode path |
| macOS | `clang --version` | `Apple clang version …` |
| All | `git --version` | `git version 2.x.y` |

**Ready when** all five baseline commands print a version and the platform rows pass —
with no application code in the tree yet.

---

## 9. Troubleshooting

| Symptom | Fix |
| --- | --- |
| `cargo` / `rustc` / `rustup` "is not recognized" (Windows) or "command not found" (Linux/macOS) | Open a **new** terminal — `PATH` is read at shell start-up. Windows: verify `%USERPROFILE%\.cargo\bin` appears in `$env:Path -split ';'` and re-run `rustup-init.exe` if not. Linux/macOS: `source "$HOME/.cargo/env"`. |
| `tauri` exists but `cargo tauri` does not work | The Cargo bin directory must be on `PATH`; meanwhile the binary can be called directly (`~/.cargo/bin/tauri -V`, `%USERPROFILE%\.cargo\bin\tauri.exe -V`). |
| `pnpm: command not found` right after `corepack enable pnpm` | Restart the terminal, or fall back to `npm install -g pnpm`. |
| `link.exe not found` or `LINK : fatal error LNK1181` (Windows) | The MSVC "Desktop development with C++" workload is missing — see §6.1. |
| `rustc -V` reports a GNU host triple on Windows | `rustup default stable-msvc`, then re-install/repair the Build Tools if it still fails. |
| `pkg-config` cannot find `webkit2gtk-4.1` / linker errors about missing GTK libs (Linux) | A `-dev` package from §6.2 is missing; re-run the apt/dnf/pacman command and re-check with `pkg-config`. |
| `Package libwebkit2gtk-4.1-dev has no installation candidate` (Linux) | The distribution is too old — WebKitGTK 4.1 needs Ubuntu 22.04+ (or the equivalent release of your distro). |
| `error: linker cc not found` (Linux) | Install the distribution's C toolchain (`build-essential` on Debian/Ubuntu, the `c-development` group on Fedora, `base-devel` on Arch). |
| `failed to run light.exe` while packaging an MSI (Windows) | Enable the optional **VBSCRIPT** Windows feature — see §6.1. |
| Blank window or graphics errors on Linux (NVIDIA, Wayland) | See the Tauri debug guide: <https://v2.tauri.app/develop/debug/>. |
| `cargo install tauri-cli` takes forever / is killed | It compiles from source; allow several minutes and ~1 GB of free disk for the Cargo cache. |

---

## 10. Verification performed for this guide (2026-09-19)

This guide was validated by running detection commands on the SDC development machine
(**Windows 11 Pro, build 26200, AMD64**) and by checking every installer/package name
against its upstream source. **Nothing was installed, upgraded or removed** — only
read-only detection commands were executed, and the only files added are the six listed at
the top of this document (plus `H:\SDC\NOTES.md`, which sits outside the project).

| Tool | State found | Action required |
| --- | --- | --- |
| Git | `git version 2.55.0.windows.2` | none |
| VS Code | installed | none |
| winget | available on `PATH` | none (used for the identifier checks below) |
| Node.js / npm | v24.18.0 / 11.16.0 | install **Node.js 20 LTS** and select it (§2) — v24 is not the SDC target |
| pnpm | **not installed** | §3 |
| rustup / rustc / cargo | **not installed** | §4 |
| Tauri CLI (`cargo tauri`) | **not installed** | §5 |
| MSVC Build Tools / Visual Studio | **not detected** (no `vswhere.exe`, no Visual Studio install directory) | §6.1, "Desktop development with C++" |
| WebView2 runtime | installed — Microsoft Edge WebView2 Runtime 153.0.4234.48 | none |

### Machine state after the STEP 1 scaffold (2026-09-19)

The scaffold step changed four rows of the table above; nothing else was installed, upgraded
or removed.

| Tool | State now | How |
| --- | --- | --- |
| pnpm | **10.34.5, on `PATH`** | `npm i -g pnpm@10` (npm's global prefix is `%APPDATA%\npm`). `corepack enable` fails on this machine with `EPERM` because it writes shims into `C:\Program Files\nodejs`; `corepack pnpm` works as a fallback but does not put `pnpm` on `PATH`. |
| rustup | **1.29.1 installed** | `winget install --id Rustlang.Rustup --exact` (the route in §4). |
| Rust toolchain | **`stable-x86_64-pc-windows-msvc`** (default, rustc 1.98.1); `stable-x86_64-pc-windows-gnu` also present | `rustup toolchain install … --profile minimal`. The msvc toolchain is the SDC target and is what everything now uses. The gnu toolchain was installed only to build the dependency-free `sdcd` binary before MSVC existed; it is no longer needed and can be freed with `rustup toolchain uninstall stable-x86_64-pc-windows-gnu`. |
| Tauri CLI | **2.11.4, as a dev dependency** of `app/` (`@tauri-apps/cli`) | `pnpm install`. `pnpm --filter app tauri …` is the primary route; §5's `cargo tauri` still works if installed globally. |
| MSVC Build Tools | **installed** | §6.1, via the VS 2022 Build Tools bootstrapper with `--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended` (one UAC consent). Result: MSVC 14.44.35207 and Windows SDK 10.0.26100.0 under `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\`. With that, `cargo run` in `sdcd/` links on the default msvc toolchain and `pnpm --filter app tauri:dev` opens the window. |

Installer identifiers were confirmed against the winget catalogue **on this machine**
(`winget search --id … --exact`), so no package name in §1–§6 is guessed:
`Git.Git`, `OpenJS.NodeJS.LTS`, `CoreyButler.NVMforWindows`, `Schniz.fnm`, `pnpm.pnpm`,
`Rustlang.Rustup`, `Microsoft.VisualStudio.2022.BuildTools`,
`Microsoft.EdgeWebView2Runtime`.

Sources used while writing this guide:

| Topic | Source |
| --- | --- |
| Tauri 2 prerequisites, platform dependency lists, rustup, Node/pnpm notes | <https://v2.tauri.app/start/prerequisites/> |
| rustup install commands | <https://rustup.rs/> · <https://www.rust-lang.org/tools/install> |
| Node.js 20 LTS installers | <https://nodejs.org/dist/latest-v20.x/> |
| macOS Homebrew `node@20` status (deprecated, 20.20.2) | <https://formulae.brew.sh/formula/node@20> |
| VS Code EditorConfig extension | <https://marketplace.visualstudio.com/items?itemName=EditorConfig.EditorConfig> |
| Linux graphics debugging | <https://v2.tauri.app/develop/debug/> |

### Documentation integration (2026-09-19)

| Check | Result |
| --- | --- |
| `docs/MASTER_SPEC.md` vs the supplied `Full Plan.txt` | identical after CRLF→LF normalisation (1,191 lines) |
| `design/ui-prototype.html` vs the supplied `UI.txt` | identical after CRLF→LF normalisation (2,543 lines) |
| `design/tokens.json` — JSON validity | parses; 97 tokens total |
| `design/tokens.json` vs prototype CSS (lines 12–53) | **97/97 tokens matched exactly, 0 mismatches** |
| dark colours vs spec §8.1 | **36/36 identical** |
| light colours vs spec §8.1 | 25 listed tokens identical; 10 exist only in the prototype (`bg-glass`, `accent-glow`, `purple-subtle`, `green-subtle`, `orange-subtle`, `red-subtle`, `diff-add-bg`, `diff-remove-bg`, `diff-add-text`, `diff-remove-text`) and are recorded from the prototype, which wins (spec §7) |
| light-theme inheritance | `state-idle` is the only dark token the light theme does not redefine |
| non-CSS values (`base-size` 13px, `line-height` 1.5, `letter-spacing` −0.005em, density, breakpoints) | taken from spec §8.2, §8.3, §8.5 and §7.2 — clearly separated in the JSON so nothing is presented as a CSS-extracted value |

**Design system (spec §8) implemented 2026-09-20.** `design/tokens.json` now uses the spec's shape —
`color.dark` / `color.light` plus flat `typography`, `radius`, `shadow` and `motion` — and
`app/src/styles/tokens.css` is its 1:1 CSS mirror (92 values: 57 in `:root`, 35 in the light block,
which is where every light override lives because `:root` is already dark). A scripted parity check
compares both blocks against the JSON in both directions and reports identical keys and values. Two
consequences of the new shape supersede the flat 97-token layout recorded above: shell metrics
(spec §7.2, `--sidebar-w` …) now live in `app/src/styles/globals.css`, and the non-CSS metadata
(density, breakpoints, skew) is left to the spec instead of being duplicated in the JSON.

The supplied `H:\SDC\Full Plan.txt` and `H:\SDC\UI.txt` were **left untouched** — the copies
under `sdc/docs/` and `sdc/design/` are now the ones to edit.

---

## 11. Next step

Everything this guide installs is the toolchain the master spec requires.

**The application scaffold is done (STEP 1).** `sdc/app/` (Tauri 2 + React 18 + TypeScript +
Tailwind) and `sdc/sdcd/` (host daemon, separate cargo binary) exist, the workspace root is
`pnpm-workspace.yaml`, and every decision taken — including why the spec's `make gen` becomes
the pnpm script `tokens:gen` instead of a Makefile — is recorded in `README.md`. What remains
is the UI layer from `docs/MASTER_SPEC.md`:

1. Wire the token generator: `design/tokens.json` → token CSS + Tailwind theme, exposed as
   `pnpm tokens:gen` (spec §4.1). `app/src/styles/tokens.css` is already the 1:1 CSS form of
   `design/tokens.json` (spec §8.1–§8.4) and `globals.css` imports it, so the generator only has
   to take over maintaining it.
2. Build the UI modules in phase order — M1 Topbar → M2 Sidebar (host-grouped) → M3 Tab
   strip → M4 Turn stream → M5 Prompt area → M8 Status bar → M7 Right panel (6 tabs) →
   M9 Command palette → M6 Model dropdown → M11 Provider Hub (spec §10).
3. Build each component from its own spec section using the spec's component contract
   (সংযুক্তি C): context → component + file location → props → states → behaviour →
   constraints → acceptance. Acceptance means parity with `sdc/design/ui-prototype.html`
   (spec §7: the prototype wins).

Known gap carried into that task: the spec defers much of itself to the v2.0 document
(§3.3–§3.6, §4.2–§4.6, the full SDCP §5, the full schema §6, the 81 non-UI modules of §10,
and §11–§22). That document is **not** in this repository, so daemon/backend work must wait
until it is supplied — the UI layer (§7–§9) is fully specified and can start right away. In
addition, `make gen` is named but not defined by the spec; the scaffold settled it as the pnpm
script `tokens:gen` (see `README.md`).

The platform blocker is gone: MSVC Build Tools and the Windows SDK are installed (§6.1, §10), so
`pnpm --filter app tauri:dev` opens the window and `cargo run` in `sdcd/` links on the default
toolchain. Every item on the STEP 1 acceptance list is now verified on this machine.

Nothing beyond the six files listed at the top of this document belongs to *this* step.
