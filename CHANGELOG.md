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

## [0.21.1] — The rest of "nothing to type": the agent's browser, servers, Linux packages, green CI

- **The agent's browser, installed by SDC.** On a machine with no Chrome, Edge, Chromium or Brave (many Linux
  desktops have only Firefox) the agent could not look at a page it built. Settings → Environment now has a
  "Browser for the agent" row whose Install fetches Google's Chrome for Testing headless shell (Windows, macOS
  Intel/Apple Silicon, Linux x64/arm64) into SDC's tools folder; an installed browser is still preferred. Verified
  live: installed, then driven by the agent's own code - page opened, read, photographed.
- **Servers too.** On a host's environment card, Node.js and the Claude Code / Codex / Gemini CLIs install into
  `~/.sdc/tools` on the server over SDC's own connection, without `sudo` (the Install there used to open a terminal),
  and every command SDC runs on a host finds them first on its `PATH`.
- **Linux packages.** A `.rpm` for Fedora / openSUSE next to the `.deb` and the AppImage; the release notes send
  Ubuntu and Debian to the `.deb`, which installs with a double-click (the AppImage needs FUSE 2 there).
- **Voice** says the no-install way first: connect Groq (a free key) or OpenAI. whisper.cpp ships no ready-made
  programs, so fully offline voice stays opt-in.
- **CI is green for the first time since 0.18.** The e2e harness stopped a relay by killing `wrangler` alone on
  Linux, leaving its `workerd` serving - "the relay goes away" never happened and the page-pairing suites failed
  there (Windows ends the whole tree, so they passed locally). The relay now runs in its own process group. The
  credential-file scan flagged `cloud/.dev.vars.example` (empty placeholders only); it is `cloud/dev.vars.example`
  now, and the scan's rule is unchanged. A test helper no longer takes a slow daemon answer for no answer, and the
  single-element loop newer clippy flags is gone.
- Tests for what 0.21.0 left unproven: a real `.tar.zst` unpacks with its program runnable (Ollama for Linux - its
  frame window was checked too), the server install script passes `sh -n`, and the server command line keeps its
  `$$` pid.
- Verified live with the owner's own Alibaba key on a copy of the data: the footer read `27.5k in · 19.7k cached`.

## [0.21.0] — All in one: no terminal, the same window everywhere, a live stream that reads like Claude Code

**Nothing to type in a terminal, on any platform.** SDC now installs what it needs itself, into its own folder
(`%LOCALAPPDATA%\sdc-tools`, `~/Library/Application Support/sdc-tools`, `~/.local/share/sdc-tools`), without
administrator rights: Node.js (LTS, from nodejs.org), the Claude Code / Codex / Gemini CLIs (npm, with that Node
when the machine has none), Ollama (the official release archive), ripgrep and Git (PortableGit on Windows - git,
bash and ssh; Apple's own Command Line Tools window on a Mac; the package manager behind the desktop's password
prompt on Linux). One **Install** button with a progress bar in the first-run wizard, Settings → Environment and a
provider's Connect card, where a command to copy used to be. Git on a Windows machine without it is fetched in the
background on first start (checkpoints need it). The tool folders are on the daemon's `PATH` from the start, so an
install is used at once; Claude Code is pointed at SDC's Git Bash when that is the only one.
- Ollama is started by SDC whenever it is installed and not running (no `ollama serve`), and a model that is not
  downloaded is fetched by SDC (no `ollama pull`) - the Research settings have a Download button.
- The doctor's **Kill process** for a busy port 3000 now frees it.
- Closing the Connect dialog with Esc over the Provider Hub left the CLI's sign-in running - holding Codex's port
  1455, so the next sign-in failed. Any way of closing it now ends the sign-in.
- Release notes say what to click on Linux instead of `chmod` / `sudo apt`.

**One window on every platform.** The native title bar (a white strip over a dark app on Windows) is gone on
Windows, macOS and Linux alike: the topbar is the caption - it drags the window, a double-click maximises - and it
ends in the same minimise / maximise / close buttons. The duplicated navigation rail is gone too.

**The live stream.** Code in an answer is coloured as it streams (the editor's own Lezer grammars, loaded on first
use; TypeScript/JavaScript, Python, Rust, CSS, HTML, JSON, Markdown, and shell by hand). Markdown tables render.
A half-written `**bold` or code span is drawn as it will be, not as raw markers, and the caret sits at the end of the
last word - in the paragraph, list item or code line being written. "Worked for 13s" is one quiet line with a small
time gauge (the breakdown on hover) instead of a boxed card with a full-width bar.

**Fewer tokens, sooner answers.**
- A file read again in the same turn, unchanged, is answered with one line instead of the whole file again.
- A sub-agent (`task`, read-only exploration) uses the provider's own fast model (Haiku, Flash, mini…) when the
  turn's model is a slower one, and falls back to the turn's model if the key is not allowed it.
- Alibaba (DashScope) gets an explicit prompt-cache mark on the system prompt and tools, and the footer says how much
  of the input any provider served from its cache (`· 12.1k cached`).
- A key that works in more than one Alibaba region is used from the one that answered fastest.
- The provider connection is opened while the person types, so the first answer after a pause does not wait for a
  TCP + TLS handshake; a kept-alive connection the provider closed (`os error 10054`) is re-sent at once on a fresh
  one instead of a "Reconnect" card and a two-second wait; the first streamed words skip the 50 ms batching.

Verified live on Windows with a "fresh machine" daemon (no Node, CLIs, Ollama, ripgrep or Git on its `PATH`): Git
arrived by itself, ripgrep, Node, Claude Code and Codex were installed from the wizard and the Connect card, Codex's
sign-in started on its own, and a real turn with the Claude Code SDC had just installed streamed a table and
highlighted TypeScript. The title bar's buttons, drag and double-click were driven with real mouse input.

## [0.20.0] — A faster agent: fewer steps, less waiting

Where an agent turn's time goes is the model's calls (6–10 s each on the route measured) - tools took 1–2 s of a
minute-long turn. So the work is *fewer steps*, and no step wasted. Measured, not guessed: the same three tasks
(a question, a fix, a three-file feature) on a real model (`deepseek-v4.1-flash` through Alibaba), the 0.19.0
daemon against this one, run side by side, twice each - 12 runs, every result correct:

| | 0.19.0 | 0.20.0 |
|---|---|---|
| Model steps (6 runs) | 52 | 37 (−29%) |
| Input tokens | 289k | 200k (−31%) |
| Wall time, total | 603 s | 472 s (−22%) |
| A question | 61 s, 8 steps | 27 s, 3–4 steps (−56%) |
| A one-file fix | 84 s | 61 s (−27%) |
| A three-file feature | 157 s | 148 s (within the noise: n = 2) |

**Pace.** The agent reads the person's own words, locally (no model call), and a plain question is told to answer
directly - no plan, no narration, no review - while a large task is told to plan first. Code identifiers
(`add(2, 3)`) are not taken for requests. The system prompt was rewritten shorter and adaptive: it asks for
independent tool calls in one reply, for grep-then-range reads, and no longer asks for a line of narration before
every call (0.18.0's prompt did, which cost time).

**Orientation.** A new turn used to start blind - the history keeps what was said, not what the tools returned - and
spent its first steps on `list_dir .` and finding the test command. It now gets the folder's top level, the project's
own check commands and what is uncommitted, in about 150 tokens (not for a quick question).

**Self-review.** A change across two files or more than 40 lines is shown back to the model once, as a diff of the
files it changed (not the person's other uncommitted work), for a strict re-read before the summary. A small change
skips it. Together with the existing completion gate (run the project's checks) this is the verification the agent
does before it says "done".

**Plumbing.** Reads inside a reply run together for every *run* of reads, not only for a reply made of nothing else
("read a, read b, edit c, read d" reads a and b at once). One HTTP client for the whole process, so the connection to
the provider stays open between steps (measured from this PC: DeepSeek 376 → 164 ms per step, OpenAI 122 → 29 ms,
Anthropic 362 → 284 ms). Anthropic requests cache the conversation up to its newest block, so each step reads the
history back from the cache instead of resending it at full price and full time-to-first-token.

**Where the time went.** A turn's footer now says it: `8 steps · 47.8k in · 2.2k out · model 2m 08s · tools 1.0s`.

**Honest limits.** The model's own speed per step is the floor; none of this makes a slow model fast, it asks it
fewer questions. The three-file feature showed no reliable gain in two runs, and the new self-review adds a step to
a multi-file change on purpose. The Claude Code, Codex and Gemini engines run their own loops: they get the pace
line, not the orientation or the cache. `sdc/scripts/tools/bench-agent.mjs` reproduces the measurement.

## [0.19.0] — A new shell and a deep live transcript

**A new layout.** A navigation rail now runs down the window's left edge: the logo (About), New chat, Search,
the sidebar and right-panel toggles (lit while open), Providers, Agency, and theme and Settings pinned at the
bottom. The top bar is a calm command bar - the machine, a centred search, the mode switch and Stop all. Chat
tabs are pills, the prompt you sent is a framed card, and the sidebar's New chat is a quiet outline (the rail
carries the primary one).

**The live transcript, Claude Code style - with everything in it.** Every step is its own row on one rail, in
order, nothing grouped away:

- **Thinking** streams into its own box while the model thinks (pink, breathing dot), then folds to
  `Thought for 3.2s · headline`; one click reads it again as Markdown.
- **Read** shows the first lines of the file the model read, numbered; **Grep / Glob / List / Search / Fetch**
  show what they found. (The SDC Agent now reports what each of these saw - the first 40 lines - on its card;
  Claude Code already did.)
- **Edit / Write** show the diff; **Bash** shows **IN** (the command, copyable) and **OUT** (streaming live).
- A failed step has a red dot and its error in red.
- The bottom line is Claude Code's: a turning ✻, a warm word for what is happening (`Running Bash…`,
  `Cerebrating…`), the clock, tokens so far, the plan step and `esc to stop`.
- Long boxes fold to their first (a file) or last (a command) lines with one click for all. The time ribbon,
  filters and Copy log sit under a finished turn.

## [0.18.1] — the 0.18.0 build, with a steady test suite

0.18.0's release build stopped at `cargo test` on three systems: two background-process tests shared the process-wide
limit of 12 running processes and raced when cargo ran them in parallel. They now take turns (a test-only lock), and
the CI workflow file parses again (three step names had an unquoted `: `). The app itself is 0.18.0's.

## [0.18.0] — Aurora Glass: a new look, a live work timeline, a sharper agent

**The new logo, everywhere.** The SDC hexagon (background removed) is the app icon on Windows, macOS and Linux
(`.ico`, `.icns`, every PNG size), the topbar mark, the empty states, About, onboarding, and the SDC Anywhere web app
(favicon, PWA icons including a maskable one, notification icon).

**Aurora Glass design.** Dark glass by default, light glass one click away. The window carries a soft light in the
logo's colours (violet, fuchsia, coral) and the sidebar, chat and right panel float on it as rounded translucent
panels; menus, dialogs and the composer are frosted glass. New palette with every text colour checked for WCAG AA
contrast in both themes; New chat, Send and primary buttons use the brand gradient.

**Premium type.** Geist for the interface, Geist Mono for code and the terminal, and Hind Siliguri for Bangla script
(Geist has no Bengali glyphs). All self-hosted.

**The live work timeline.** A turn's work is now one rail with a dot per step, coloured by how it went: `Thought for
3.2s` (one click opens the reasoning as Markdown), `Explored 2 files, 1 search` with each read listed, `Edit` with its
diff right under it, and `Bash` with **IN** (the command, copyable) and **OUT** (the output, streaming live, folded to its
last lines when long). The bottom line always says what is happening this second - a turning spark, the phase, the
clock, tokens so far, the plan step and `esc to stop`. The time ribbon, pace sparkline, filters and Copy log stay. The
person's message is a bubble, the answer reads as prose under an SDC header, the plan card has a progress bar.

**A sharper agent.** SDC Agent's brief now asks for a senior engineer's method: restate the goal and what "done" means,
ground every claim in the project's own code, find a bug's root cause before fixing it, think through edge cases,
narrate each step in one line, review its own change before answering, never claim a test passed without running it,
and finish with a structured summary (result, changes, cause, verification, what is left).

**Fixed.** Colour classes with an opacity modifier (`border-accent/30` and eight others) were never generated, so those
borders fell back to a light grey; they now have their colours, and a bare `border` defaults to the subtle hairline.

## [0.17.0] — SDC Anywhere, phase 1: a safe foundation

Control this computer's SDC from any browser, with nothing to install. **Off by default.** With it off, SDC behaves
exactly as before; with the relay down, the desktop is unaffected.

**What a person can do.** Pair a phone or a borrowed laptop with a QR code and a passkey, compare six digits on both
screens, confirm on the computer. From the browser: see the live stream, see and answer permission requests (Allow
once, Deny with a reason the AI receives), stop everything (Stop all, no unlock needed). Settings → SDC Anywhere on
the desktop turns it on, pairs, lists and revokes devices, and sets the limits. See `docs/remote/`.

**How it is built.** The PC dials out to a relay (a Cloudflare Worker and one Durable Object per computer); nothing
is opened on the PC. Everything between browser and daemon is end-to-end encrypted (HPKE, X25519 and AES-256-GCM, a
counter-numbered context per direction), so the relay forwards ciphertext it cannot read, alter or replay. A device is a
non-extractable ECDSA key plus a passkey; the daemon verifies the WebAuthn assertion itself. An approval is a signature
over the SHA-256 of the exact action shown; the daemon compares it with its own copy before anything runs.

**Rules the code enforces** (not settings): four levels - locked, view (passkey, locks after 15 idle minutes), operate (passkey,
5 minutes), and a fresh passkey for every dangerous action; a browser may call only an allow-list of daemon methods and
never `anywhere.*`, policy, keychain, SSH keys or audit; a guest is view-only for at most 2 hours; revoking a device cuts
its session at once. A connection that drops comes back at the same level for 2 minutes without a passkey.

**New in the daemon.** `sdcd/src/anywhere/` (`core`, `crypto`, `router`, `session`, `webauthn`, `registry`, `relay`, `os`);
ten `anywhere.*` methods and the `RemoteActivity` event; the ledger records which device decided and why; `SDC_DATA_DIR`
moves the data folder; `Decision::DenyWith` carries a refusal's reason to the model; the Windows/macOS/Linux idle and
keep-awake hooks. New dependencies: `hpke`, `p256`, `getrandom`, `tokio-tungstenite` (rustls), `futures-util`, `windows-sys`.

**New packages.** `cloud/` (the relay), `web/` (the PWA; English and Bangla so far), `bench/remote/`.

**Measured** (relay, daemon and browser on one machine, so software overhead only): an approval reaches an open page
in 5.5 ms p50 / 6.9 ms p95, Allow round trip 7.3 / 8.5 ms, a stream event 2.5 / 2.8 ms, reconnect 554 / 635 ms, a
256 KB frame seals in 0.5 ms, the daemon grows 12.6 MB. `docs/remote/PERF.md` has the method and what is not measured yet.

**Tests.** 160 new Rust unit tests (crypto, WebAuthn, router, capability levels, the whole core against a software
browser), 31 relay tests on workerd, 16 TypeScript vector tests that must agree byte for byte with Rust, 25 end-to-end
tests with a real `sdcd`, the real Worker and the real page in Edge with a virtual authenticator (including axe
WCAG 2 AA checks).

**Not in this release** (by the plan): the file explorer and editor (0.18/0.19), terminal, preview, push and e-mail
notifications, the magic link (0.18), WebRTC (0.20).

## [0.16.1] — Each chat keeps its own model, local models really run, and `/research`

**The report:** *"akta chat a je model select kora onno chat onno model select korle aita automatic sob
chat thake model change hoi"*. The window held one model for every chat, so a pick in the VPS chat's box
changed the local chat beside it.

**Each chat keeps its own model.** A pick belongs to the chat it was made in and is remembered across
launches. A chat with no pick of its own shows the model its last turn ran. A new chat starts from the
latest pick, and the chats already on screen are pinned first so they do not move. In split view only
the clicked box opens its menu. Fix with agent, Connect's `Use`, Alt+M, Alt+E and the status bar act on
the open chat only.

**Local models (Ollama).**
- A model picked from the Local group was routed as an API turn. That route skipped Ollama and failed
  on `custom`'s missing key. It is now the local engine's turn.
- The agent talks to Ollama's own `/api/chat` and names its context (`num_ctx`) on every request.
  Before, it used `/v1`, which cannot set the context, so Ollama ran at its 4 096-token default while
  SDC planned for 32 000. SDC's window for a local model is that same number: the catalogue's `ctx`,
  capped at 16 384 by default (Settings → Research → Local model context).
- Tool results, page text and search results are cut to fit a local model's window.
- A model that is not downloaded, or a machine out of memory, gets a sentence that says what to do
  (`ollama pull …`, a smaller context).
- `qwen3.5:9b` is in the catalogue.

**`/research <question>`.**
- The question is answered from the web, with numbered sources `[n]` (title, address and date) listed
  under the answer.
- Before it starts, a card shows the model and where it runs, the search service, the limits and the
  estimated cost.
- A research turn has its own tools (search, fetch, sub-agents, plan, read) and limits on searches,
  pages and minutes. A local model's cost governor sees $0, so the limits are what stop it.
- Search services: DuckDuckGo (default, no key), SearXNG, Tavily, Brave and Serper. Keys are kept in
  the OS keychain. A keyed service that fails falls back to DuckDuckGo with a note.
- Pages are read for their article: the main text, title and published date, not the menus.
- Optional: an API model can write the final answer from what a local model gathered.
- Every model searches the web when a question needs it, local models included, through the search
  service chosen here. Saving a key while DuckDuckGo is in use switches to that service, so a Tavily key
  is all it takes. Settings → Research can keep a local model off the web except through `/research`;
  it is then not offered the web tools at all. A `privacy = "local-only"` folder opens the web for a
  research turn alone.
- Every agent turn is told today's date, so searches use the current year. A research answer drawn from
  one page is asked, once, to read a second source.

**Live-checked** against Ollama 0.35 with `qwen3.5:9b` on an RTX 4060 (8 GB) and Tavily:
- `ollama ps` showed context 16384, so `num_ctx` arrived.
- The local agent read a file and answered in 8 s.
- `/research` with a Bengali question ran 2 Tavily searches, read 2 pages and answered in Bengali with
  `[n]` citations and 9 sources, in about 2 minutes.
- The local model looked up today's weather by itself.
- DeepSeek's `web_search` went through Tavily.
- `/research stop` stops it. `/model <name>` picks this chat's model.

**Review fixes:**
- Ids can no longer repeat and replace rows.
- The file guard follows symlinks, locally and on hosts.
- Events the disk refused are kept and written later.
- Passwords keep their spaces.
- Masking a key with a non-Latin character no longer panics.
- A panicking connection no longer holds the daemon open.
- Slow handlers run on the blocking pool, and a reader that stops reading is let go.
- A team name that is not on the team no longer locks the owner out.
- The deploy duplicate check matches the exact site.
- A stop that cannot reach the host is reported.
- Forward rewind stays in its own chat.
- The WebSocket transport fails waiting calls when the socket closes.
- The background poll no longer toasts every second.
- A folder is never bound to a chat on another machine.
- An error without a turn becomes a toast.
- The bridge retries a failed subscribe.

## [0.16.0] — SDC holds the VPS connection itself, and can stay signed in for good

**The report:** *"ami cai ai rokom sign out jano kono vabai nah hoi"*. The VPS signed out six times in one
day, about every 90 minutes.

**The cause was on this PC, not on the VPS.** The connection was an `ssh -f -N` ControlMaster from Git for
Windows. The process was found at 89% of a core, alive, holding its TCP connection and serving nothing
(`muxclient: master hello exchange failed`). 0.15.4 and 0.15.6 could only notice it and kill it, and every
kill was a sign-out.

**SDC's own SSH connection (`ssh::native`, `russh`).** The daemon now signs in and holds the connection
itself. The ssh.exe master process is gone.

- One connection per host. Every command, file read, a turn's CLI, an MCP server, the Terminal and the
  preview's port forward run as channels on it.
- A keepalive every 15 s. Only two minutes of silence end the connection.
- At most 9 channels at once, below OpenSSH's `MaxSessions 10`. A burst waits for a free slot instead of
  being refused. Live check: 16 commands at once on the real VPS, all answered in 1.4 s.
- When the connection drops, SDC signs in again by itself if it can. A command sent during that wait
  runs once the new sign-in finishes instead of failing.
- A host SDC is signed out of is answered "sign in again" on this PC, without dialing. Each refused dial
  was a `PerSourcePenalties` strike against this PC.
- The first sign-in ends the stuck master an older SDC left behind.
- Streaming callers start `sdcd --ssh-bridge`, a process that behaves like `ssh`, so nothing that starts,
  reads or stops them had to change.
- The old master stays as a fallback, only for a host whose key exchange the new client cannot do.

**Stay signed in.** A new box on the Sign in card, on by default. SDC keeps the password and the
authenticator's setup key in the OS keychain and makes the 6-digit code itself (RFC 6238, `ssh::totp`).
With it on, a dropped connection or a restarted SDC signs in again with nothing typed. Leave the key field
empty and SDC reads it from `~/.google_authenticator` on the host, over the connection that was just
opened. Turning the box off forgets the key.

**Other fixes found on the way:**

- **Files panel.** `listing failed … Permission denied` stayed red under FILES after a sign-in or a
  Reconnect. The tree, the open folders and the git badge now reload as soon as the host is `connected`.
- **Sign in card.** The card opened without password and code fields when the host's message was
  `master hello exchange failed`, the message in the report. It also missed every new "dropped" sentence.
  Now every sentence that means "sign in" opens the fields.
- **Stop all.** It killed only the direct child process: a dev server's `node` under `npm`, or the command
  on the VPS, kept running. Each process is now stopped the way its own Stop button stops it: the whole
  tree, and the remote process too. The toast also said "Nothing was running" right after stopping a
  process. It now counts processes.
- **Simple | Pro | Auto.** The switch never said what it does. It sets how often SDC asks before it acts.
  Each mode now has a tooltip and a toast that say so, in English and Bengali.

## [0.15.12] — SDC stops knocking on a VPS it is signed out of, and a cleaner prompt box

**The report:** the VPS kept "restarting". Its own logs said otherwise: the server was healthy (37 days
up), and what dropped was the SSH connection. The log showed `Connection closed by authenticating user …
[preauth]` from this PC several times a minute, `Unable to negotiate … sk-ssh-ed25519`, and an OpenSSH
10.2 `PerSourcePenalties` penalty on the user's own address block.

**Two of those came from SDC.**

- With the signed-in connection gone, every file read, `git status` and probe dialed the VPS again with
  SDC's key. A host that also wants a 2FA code refuses the key every time, and each refusal counts against
  this PC. Enough of them and the VPS turns away the real sign-in with a fresh code. Now, after one refusal,
  SDC does not dial that host with the key again for 5 minutes. It answers the same "sign in again" without
  touching the network. A sign-in or a key install clears this at once.
- `ssh-keyscan` asked for the FIDO key types (`sk-ecdsa`, `sk-ssh-ed25519`) too, one connection each. The
  server has neither, so each one ended before authentication. The scan now asks only for Ed25519,
  ECDSA and RSA.

**What SDC cannot fix** is on the server and the network: about 220 ms and 5.7 % TCP retransmits on the
path, sshd with `ClientAliveInterval 0` and `LoginGraceTime 30`, and `needrestart` restarting services
around 06:00. Those are settings on the VPS.

**0.15.11 was never published**: its Windows build stopped on a PowerShell test that ran out of time on a slow CI runner. That test now waits 60 s instead of 20 s.

**The prompt box.** The text field drew its own focus outline, a second, smaller box inside the prompt
box. The prompt box is now the only focus indicator. It is also rounder and has more room, a softer
focus glow that follows the light and dark themes, a slightly larger Send button, and a fade between the
conversation and the box.

## [0.15.10] — A VPS connects from Windows again, and every platform passed the same end-to-end run

**The report:** *"after the update the VPS does not connect."* The host card said `refused the sign-in:
command-line line 0: invalid quotes`.

**The cause was 0.15.8's Mac fix.** To survive macOS' `~/Library/Application Support`, it wrapped every
`ControlPath=`/`UserKnownHostsFile=` value in quotes. On Windows, Rust passes an argument **without** a space
as-is and escapes its `"` as `\"`, and Git for Windows' Cygwin `ssh` keeps the backslashes, so every VPS
sign-in on Windows failed. 0.15.8's test had tried only a path *with* a space, which Rust quotes differently
and which passed. The quotes are now added only when the path has a space. A second test runs the ordinary
Windows path through the real `ssh` from Rust, as SDC does, so both shapes are proven.

Found by a new end-to-end run, `os-probe`, that does the same thing on Windows, macOS (Apple Silicon) and
Ubuntu from source. It starts the daemon with the bare environment a Dock or desktop launcher gives, then
checks: the CLIs are found, the doctor, a project in a folder with a space, file write/read/list, the Terminal
and its Commands, a CLI sign-in link, a **real Claude Code turn and a real Codex turn** (each must end with an
answer or a clear sentence, never hang), and on Linux and macOS a real SSH host (add → trust the key → sign in
→ remote files → remote terminal → doctor → remove). On Linux, the `.deb` is installed and must start its own
daemon. It also found:

- **The Terminal's Commands did not run in a local chat on Windows.** A local chat carries `hostId: "local"`,
  which `shell.run` took for a remote host and sent to `sh -c`. Windows has no `sh`.
- **The Environment doctor said "SSH: not installed" on every platform.** It ran `ssh --version`, which is not
  an option (OpenSSH has `-V`, on stderr). It now asks the `ssh` SDC really uses (Git for Windows' on Windows).
- **A Codex turn ended at its first retry.** Codex prints `Reconnecting... 2/5` as a top-level error while it
  keeps trying, and SDC took that as the end. A turn that would have recovered on a flaky network was
  stopped, with "Reconnecting... 2/5" as the reason. Retries are now warnings, and only `turn.failed` ends it.
- **A signed-out Codex now says so.** "Codex is not signed in - Providers → OpenAI → Connect" replaces a 401
  with a websocket URL and a Cloudflare ray id.

**Results of the last run:** Windows 16/16 (the runner has no SSH server; the Windows ssh path is covered by the
new test, which runs the real Git `ssh` the way SDC does), Linux 26/26 plus the installed `.deb` started its own
daemon, and macOS 26/26. On macOS the SSH host signed in with SDC's own key, because GitHub's Mac runner does not
accept password logins, and a plain `ssh` with the right password was refused there too. So the kept-open
connection under `Application Support` is proven on a Mac, and typing a password or 2FA code on a Mac is not.

## [0.15.9] — Sign in works on a new Mac: the Provider Hub opens on top of the Welcome dialog

The report, on Apple Silicon after updating to 0.15.8: *"Sign in does nothing."*

Reproduced on a real Apple Silicon Mac (a GitHub `macos-14` runner, with the window driven through the
accessibility API and screenshotted after each step). On a fresh install the **Welcome to SDC** dialog opens
first, and its "Open the Provider Hub" opened the hub **underneath** it. `<Onboarding />` was rendered last,
so it covered every dialog it opened. The provider cards could be seen, blurred, and could not be reached.
It looked like nothing happened. This never shows on a machine that finished the Welcome dialog long ago,
which is why no Windows check caught it. The Welcome dialog now steps aside while the Provider Hub, Connect,
Add a server, Settings or New project is open, and comes back on the same step. On the same Mac, pressing the
Claude card now opens Connect, starts `claude auth login`, shows the link and the code box, and Safari opens
Claude's Log in page.

Also:

- **Approvals are always on top.** The Permission dialog, where a turn waits for your yes, was rendered under
  Connect, Agency, Cost Center, the policy editor, the memory dialog and the Welcome dialog. It is now the
  last dialog drawn.
- **Codex was shown as connected without a sign-in.** `codex login status` prints "Not logged in", which
  contains "logged in". It is now read as signed out (seen on the clean Mac, with a test).
- **The CLI status checks are bounded.** `claude auth status` and `codex login status` run inside the
  provider list, and a CLI that waited on something held the list, and the window, with it. They now give up
  after 15 s.
- **One browser tab, not two.** 0.15.8 opened the sign-in page itself on macOS and Linux, but the CLIs
  already do that (measured on the Mac: Claude and Codex both open Safari). The page opened twice, so SDC's
  own opening is removed. "Open in browser" in the dialog still works.

Checked on the real Apple Silicon Mac against 0.15.8's installer. With launchd's bare `PATH`
(`/usr/bin:/bin:/usr/sbin:/sbin`), `SHELL=/bin/zsh` and the CLIs reachable only through `~/.zshrc`, the
daemon found `claude` and `codex` and started both sign-ins. This fix, built from source on the same Mac,
was then pressed through in the window, from the Welcome dialog to Claude's Log in page.

## [0.15.8] — macOS and Linux work like Windows: CLIs are found, links open, copy and paste work, a VPS connects, and every platform ships the same version

The report: *"the way it performs on Windows, it doesn't perform on macOS or other OS. On macOS the browser
does not open automatically, and even the link option in the CLI sign-in - click it or copy it - does not
work. The API has issues too."* Every cause below was Windows-shaped code that no Windows check could fail.

**The CLIs were never found on a Mac.** An app opened from Finder or the Dock gets `launchd`'s `PATH`
(`/usr/bin:/bin:/usr/sbin:/sbin`). `claude`, `codex`, `gemini`, `node` and `npm` live in Homebrew, npm,
`~/.local/bin` or nvm folders, so the daemon said "not installed" or "not connected" about CLIs that ran fine
in Terminal. No sign-in could start, so no browser opened, and the agent's `npm`/`node` commands and the
preview's `npm run dev` failed the same way. `sdcd` now asks the user's login shell for its `PATH` at start
(bounded at 4 s) and adds the usual install folders that exist (Homebrew, `~/.local/bin`, npm-global, bun,
volta, pnpm, the newest nvm Node; on Windows `%APPDATA%\npm` and the Node folders). Linux desktop launchers
had the same hole for nvm and `~/.local/bin`.

**Links and the clipboard did nothing outside Windows.** The sign-in dialog's "Open in browser" was a
`target="_blank"` link, the subscription flow used `window.open`, and Copy/Paste used `navigator.clipboard`.
Windows' WebView2 honours all three. macOS' WKWebView and Linux' WebKitGTK silently ignore them. They now go
through Tauri's native opener and the new clipboard plugin (`copyText`/`readText`), and any other web link in
the window is sent to the browser too. On macOS and Linux the CLI's sign-in page now opens by itself as soon as
its link appears. A copy that still fails says so instead of doing nothing. ESLint now rejects `window.open`,
`navigator.clipboard` and `target="_blank"`, so they cannot come back.

**No VPS could connect from a Mac** (or from a Windows account whose name has a space). The ssh options
`ControlPath=` and `UserKnownHostsFile=` carried the data folder unquoted, and on macOS that is
`~/Library/Application Support/sdc`. OpenSSH refused the first (`keyword controlpath extra arguments at end
of line`) and split the second into two files, so the host key never matched. Both are quoted now, with `%`
escaped. A test runs the real `ssh -G` against a path with spaces.

**Stop did not stop anything on macOS or Linux.** `kill_tree` was a no-op there, and a stopped Claude, Codex
or Gemini turn went on running. So did an agent's `sh -c "npm run dev"` children and the preview's dev server.
Every child SDC can stop now starts as its own process group, and the whole group is ended, as `taskkill /T`
already did on Windows.

**API keys on a Mac.** The Keychain asks before an app it does not recognise reads an item, and an ad hoc
signed build is "new" after every update. SDC read the Keychain on every provider list and every turn, and a
declined question read as "No API key". Keys are now read once per daemon start, and a refused read says so
("choose Always Allow when the Keychain asks about sdcd, or connect the key again").

**Claude's sign-in reaches a VPS from a Mac too.** Claude Code keeps it in the Keychain
(`Claude Code-credentials`), not in `~/.claude/.credentials.json`, so a Mac forwarded nothing. It is now read
with Apple's `security` tool (bounded at 10 s).

**Also:** shortcut hints read `⌘`/`⇧` on a Mac.

**One version on every platform.** A release used to go public platform by platform, and the Windows job then
deleted every older release. A failed macOS build left a public version with no `.dmg`, and the previous
`.dmg` was already gone. `release.yml` now builds into a **draft**. A final job makes it public only when all
four builds succeeded and all six installers are attached, and only then removes the older releases. A failed
platform leaves the previous release public for everyone.

Checked: 430 daemon unit tests + 19 integration tests + 227 app tests on Windows. `cargo clippy -D warnings`
for `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu` (with the keychain feature), compiled from Windows
with zig as the C compiler. The ssh quoting against the real OpenSSH 10.3. Not run on real Mac or Linux
hardware: the macOS/Linux behaviour is compiled and unit-tested, not clicked.

## [0.15.7] — The macOS build runs on Apple Silicon without "is damaged and can't be opened"

The report: opening `SDC_0.15.6_aarch64.dmg`'s app said *"SDC is damaged and can't be opened. You should
move it to the Trash."* — the same message even after clearing the download's quarantine flag.

`tauri.conf.json` set no macOS signing identity, so the bundler shipped the `.app` **completely
unsigned**. Apple Silicon refuses to run any executable with no code signature at all, ad hoc or
otherwise — Intel Macs are more lenient and would have shown the ordinary "unidentified developer"
prompt for the same unsigned build, which is why this was not caught before. Setting
`bundle.macOS.signingIdentity` to `"-"` makes the bundler ad hoc sign the `.app` (and the `sdcd` sidecar
inside it), which is enough for macOS to run it.

This is still not a paid Apple Developer ID, so Gatekeeper's first-run prompt is unchanged: right-click →
Open, or System Settings → Privacy & Security → Open Anyway. What changes is that the app now actually
launches afterwards, on both Intel and Apple Silicon.

**If SDC_0.15.6_aarch64.dmg still sits on a Mac:** re-download 0.15.7 instead - and if the same dmg is
kept, `xattr -cr /Applications/SDC.app` alone will not fix a truly unsigned binary; it also needs
`sudo codesign --force --deep --sign - /Applications/SDC.app` to become one at all.

Not verified on real Apple hardware - this build was made and tested from a Windows machine that has no
Mac to run the resulting `.dmg` on. The config change is confirmed by `tauri info` parsing it without a
schema error, and by testing the underlying Gatekeeper rule (unsigned arm64 refuses to launch, unsigned
x86_64 launches with a warning) independently, not by launching this build itself.

## [0.15.6] — A signed-in VPS is not signed out for a slow answer, long turns are not stopped as loops, and a filtered reply is asked again

The report: in one evening the VPS was signed out three times, three long page-build turns were stopped
as a "loop", and two turns ended on `native_api failed: <400> InternalError.Algo.DataInspectionFailed`.
The daemon's event log shows the causes:

* **SDC itself closed the VPS connection.** All three sign-outs came from 0.15.4's stuck-master check
  ("stopped answering for 45 s"). Two of them happened while nothing ran on the host: a turn was waiting
  on a permission card, and later the window was idle. The PC was awake (no sleep in the System log), and
  the next master answered `-O check` in 45 ms. A few `-O check`s that ran out of time were enough to throw
  away a password-and-code sign-in. Now:
  * a master is suspected only after 9 unanswered checks in a row **and** 45 s;
  * before anything is ended, the master is asked to run `true` on the host (20 s). A master that still
    runs a command is kept;
  * any command that goes through the master counts as proof it works, so no `-O check` is spawned
    against a master that carried a command in the last 15 s.

  A master that really spins (the 0.15.4 case) cannot run `true` either, so it is still ended.
* **Five edits of one file were called a loop.** The loop guard counted `Edit page.tsx` five times in a
  row as "nothing changing between", although each edit changed different lines (the diffs are in the log).
  An edit that changed the file now resets the count. The guard also stopped any turn at 150 tool calls,
  the same kind of limit 0.15.2 removed for steps. That cap is gone; a budget in Settings is the bound.
* **The provider's content filter ended the turn.** Alibaba Model Studio checks each reply and blocked one
  that quoted long logs (`DataInspectionFailed: Output data may contain inappropriate content`). That is
  the provider's filter, not the network or SDC. The SDC Agent now asks the step again with a note to reply
  shorter and summarise instead of quoting, up to 3 times in a row, and shows a "Content filter" card while
  it does. If the provider still blocks the reply, the error card explains this in plain words (rule
  `content-filter`), not as "no rule yet".
* **Faster: reads in one reply run together.** When a reply only reads (`read_file`, `grep`, `glob`,
  `list_dir`, `search`), the reads run at the same time. On the report's VPS, four reads took about 1 s
  instead of about 4 s. A reply that mixes reads with edits still runs in order.

Checked live on the VPS through the signed-in connection: 4 reads started within 2 ms and ended within
1 s, and five edits of one file in a row ran without a loop stop (0.15.5 stops at the fifth).

## [0.15.5] — An uninstaller never erases your data, and a daily copy of it

Installing 0.15.4 over 0.15.3 with `setup.exe /S` (a silent install without `/UPDATE`, run by hand)
made the installer run 0.15.3's uninstaller as a full uninstall. SDC's own uninstall step then erased
`%APPDATA%\sdc` (every chat, host and setting) and the API keys, although nobody ticked "Delete the
application data". The in-app updater passes `/UPDATE`, which that step always skipped. Nothing else in
0.15.4 is affected.

* **The uninstaller no longer erases anything.** During an update, a silent install or a passive install
  it does nothing. On a real uninstall with the box ticked, it only moves the folder aside to
  `%APPDATA%\sdc-removed-<number>`. Renaming it back restores everything. It never removes API keys:
  Settings → Erase all data is the way to do that on purpose.
* **A daily copy of the database.** Every time the daemon starts, it keeps one consistent copy per day
  (SQLite `VACUUM INTO`) in `%LOCALAPPDATA%\sdc-backups` (`~/.local/share/sdc-backups` on Linux), the
  newest three days. That folder is outside the data folder and outside the install folder, so neither an
  uninstaller nor a deleted data folder takes it. Erase all data removes these copies too.

Note: the uninstaller that runs during an update is the *old* version's. Update from 0.15.4 through SDC's
own update (or run the installer normally, not with `/S`), so the 0.15.4 uninstaller is never asked to
remove data.

## [0.15.4] — A dropped network is retried, a stuck VPS connection is noticed, and the preview shows the page being written

The reports: an Alibaba turn of 19 minutes and 183 steps failed on one network error; a Claude Code
turn on the VPS sat on *"Waiting for sonnet's first word"* for minutes; the preview never showed the page
being worked on; and *"model change korle abr prothom thake suru korbe?"*

* **A network or provider hiccup no longer ends the turn.** Every API model (Alibaba, DeepSeek, OpenAI,
  Anthropic, Groq and the rest, in chat and in the SDC Agent) asks again when the connection drops, times
  out, or the provider answers 429 / 5xx / "overloaded": after 2, 4, 8, 15 and then 30 seconds, nine tries
  in all. A **Reconnect** card in the stream shows each try. Every step before the drop is kept. A bad key
  or an unknown model still fails at once. The turn that failed was `os error 10060` from
  `dashscope-us`: measured afterwards, the host answers from here in 0.4 s, so the drop was momentary.
* **A stuck VPS connection is found and ended.** The waiting turn was not slow. The `ssh` master holding
  the VPS sign-in was spinning at 91% of a CPU core and answered nothing. Every check of it ran out of
  time, which SDC read as "busy" on every pass, so the turn waited indefinitely. Now a master that answers
  nothing for 45 seconds is called stuck: SDC ends it, marks the VPS "sign in again" and says why. The
  waiting turn then stops with that sentence at once (checked on the report's own chat: once the process
  was gone, the turn ended in seconds). A master whose socket refuses is ended the same way, not left
  spinning.
* **A connection failure is not retried as a lost conversation.** "The earlier conversation could not be
  resumed; starting it again" no longer runs a second, useless try when the host itself could not be
  reached.
* **Switching models goes on from where the work stopped.** A turn that stopped part-way (you pressed
  Stop, the provider dropped, SDC's Trust Kernel stopped it, or it was cut off) now says so in the record
  that the next turn reads, whichever model takes it: what happened, that it really is on disk, and to go
  on from the last step rather than begin again. A turn that failed after writing some text was stored as
  a success; it is now stored as an error.
* **The preview can show the page as it is written.** The preview did follow the page (for example
  `/email-marketing-agency-uk`), but the site is a built Next.js app, and a page whose source was just
  written is a 404 there until it is built and restarted. Now:
  * the preview says so above the frame (*"answers 404 on the live site: the live site shows the last
    build that was deployed"*) instead of leaving a 404 or the home page unexplained;
  * the new **Dev** button runs the project's own `npm run dev` (on the VPS for a VPS chat), bound to
    `127.0.0.1` on a free port from 3100 and reached through the signed-in connection. The preview then
    follows the page being edited with its real styles and components, and hot-reloads as files are
    saved. The live site and its pm2 process are not touched (Next.js 16 keeps `next dev` in
    `.next/dev`). Press Dev again to stop it.
* **Faster start of a VPS turn.** The project conventions and the long-task memory are read from the
  host at the same time instead of one after the other.

## [0.15.3] — The Windows installer for 0.15.2

0.15.2 was published for macOS and Linux only. Its Windows build stopped on the same test as 0.15.0: a
`[Y/n]` answer typed while a cold PowerShell was still starting on a busy build machine never arrived. 0.15.1
said the answer again once, but only if nothing on the screen had moved, and the Windows console redraws
the screen even when nothing is typed. Now SDC looks at the last line. While it still ends on the question
with nothing typed after it, SDC gives the answer again every three seconds, up to five times. Everything
in 0.15.2 is in this release.

## [0.15.2] — No step limit, and a signed-in VPS that stays signed in

The report: a long agent turn on the VPS stopped with *"I stopped after 60 steps, the limit for one
turn"*, and the VPS showed "signed out" right after. The correct password and code on the Sign in card
did not bring it back. *"kono rate limit to dorkar nai ... kono disconnect issue kono vabai jano nah hoi."*

* **A turn has no step limit.** The SDC Agent kept going for 60 model calls and then asked for
  "continue". A turn now runs until the work is done. Stop, the cost budget (Settings) and the runaway
  detector (the same call over and over) still end a turn that goes wrong. `sdcd run --max-steps` still
  sets a limit when you want one.
* **The VPS connection is not dropped by SDC itself.** The log showed the `ssh` master still running
  and still connected to the VPS. Only its socket file was gone, one second after the turn ended, and
  from then on SDC had no way to use the connection. Three changes:
  * Right after sign-in SDC makes a second name for the socket (a hard link). If the file is ever
    removed, SDC puts it back from that link and the connection works again. No new sign-in and no
    new code are needed.
  * A connection check that is only slow (a busy PC at the end of a long turn) no longer counts as
    "closed". SDC counts it as lost only after it is really gone twice, a few seconds apart. Commands
    keep using the connection while it is slow, instead of falling back to a key the server refuses.
  * A socket file is only removed when its master is gone. Before, a slow check before a sign-in
    could remove the socket of a live connection.
* **Sign in on a host's card always does something.** When the window's copy of the host had no
  address, the button sent nothing and said nothing. The card now sends the host's id, and the daemon
  takes the address from its own record. The "already in the host list" message no longer appears on
  every sign-in.

A connection that really drops (the network goes down for more than two minutes, or the PC sleeps)
still needs the verification code once more. The server asks for it on every new connection, and SDC
cannot type it for you.

## [0.15.1] — The Windows installer for 0.15.0

0.15.0 was published for macOS and Linux only. Its Windows build stopped on one test: a sign-in question
(`[Y/n]`) answered while a cold PowerShell was still starting on a busy build machine, where the
answer never arrived. The test passes every time on a normal PC (5 of 5), but a real Gemini sign-in can
meet the same moment. So **an answer that was not taken is given once more**: if nothing on the screen has
moved for three seconds after the answer, and it still ends on the question, SDC types the answer again.
Any new output means the first answer was taken, and then nothing is repeated. Everything in 0.15.0 is in
this release.

## [0.15.0] — Flow: the live stream as a timed rail, and chats that load on a cold start

The report: *"live stream ... aro better and advance and featurefull"*, better than what the other AI tools
show. Two styles were built and tried side by side in the real window on real chats: **Flow** (a timed
rail) and **Console** (a mission log). The person asked for the better one, which is Flow, and Console's
best parts moved into it.

* **Every step on one rail, with its time.** Thinking, reading, commands, edits, the agent's words and
  your messages sent mid-run sit on one vertical rail, in the order they happened. Each step shows its
  measured duration and a done/failed mark. A click opens the full thought, the command output, the diff
  or the list of files it read. A thought still going on shows its title and its newest lines under it.
* **The time ribbon: where the turn's time went.** One bar for the whole turn, coloured by kind:
  thinking, writing, reading, editing, commands, and the grey stretches *between* steps, where the model
  is choosing its next move. That gap is shown by no other agent UI. Measured on a real 1 m 44 s turn, it
  was 80 % of the time. Each colour has its total and share. A click on a stretch takes you to its step.
* **Now.** While the turn runs, the bottom of the rail says what is happening this second (a command, the
  pause before the next step, waiting for the model) with its own clock. The head shows the output pace
  over the last forty seconds as a sparkline, the tool calls so far, and the files changed so far (+/−).
* **Filters and Copy log.** A run of six steps or more gets filters with counts: All, Thinking, Reads,
  Edits, Commands and Messages. The copy button puts the whole run on the clipboard as text, with each
  step's time offset, which is useful for a bug report.
* **Folded when done.** A finished turn folds to one line (`Worked for 1m 44s · 10 steps · 1 file +7 −0`)
  and its ribbon, and the answer stands on its own below it. Old chats are drawn the same way. The time a
  turn ended is now kept from the log (`TurnCompleted`/`ErrorRaised`).
* **Chats no longer open empty after a cold start.** The window asked the daemon for the history before
  `sdcd` was listening, swallowed the error, and never asked again, so every chat read "Nothing here yet"
  until a reload. It now asks again every second until the daemon answers. Found while testing this
  release in a fresh window.

## [0.14.4] — Alibaba keys land where they work, a dropped VPS signs in again at once, and the preview follows the page

Three reports in one message, each measured before it was changed.

* **Alibaba Cloud: "key diye add korle kaj korse nah", and DeepSeek and other models were gone from the
  list.** The key saved on the Alibaba Cloud card was a **Coding Plan** key. Model Studio refuses it (401)
  on every region - Singapore, US and Beijing - and it answers only at `coding-intl.dashscope.aliyuncs.com/v1`,
  with its own ten models. `provider.save` never checked a key, so the card said Connected, the live list
  failed, and the menu fell back to the fifteen bundled rows, which had no DeepSeek in them. Now:
  * **Alibaba Coding Plan is its own card** (`qwen-coding`), with its own endpoint, key and model list,
    separate from Alibaba Cloud (Model Studio), which lists Qwen, DeepSeek, Kimi and GLM.
  * **A key is checked before it is saved.** A key the provider refuses is not saved, and the reason is
    shown in the provider's own words. A key that cannot be checked (no network) is saved as before.
    The Coding Plan's `/models` answers 200 to **any** key (measured with `sk-bogus-0000`), so a Coding
    Plan key is checked on its chat route with an empty request: 401 for a refused key, 400 for an
    accepted one, and no tokens spent.
  * **An Alibaba key finds its own home.** SDC asks each Alibaba endpoint once: Model Studio Singapore,
    US and Beijing, and the Coding Plan in Singapore and Beijing. A Coding Plan key pasted on the Model
    Studio card is saved on the Coding Plan card (the dialog moves there and says so), and a US or Beijing
    key gets its region's URL. The endpoints are asked at the same time: 1.4 s, down from 13.6 s one
    after another.
  * **A key already saved on the wrong card moves by itself** at start-up, but only if its card refuses it
    and the other card has no key of its own.
  * The bundled Alibaba Cloud list now includes DeepSeek V4 Pro, V4 Flash, V4.1 Flash and V3.2, so they
    show even before the first live refresh.
* **A dropped VPS connection: the dialog showed "Not connected" with no password box, and signing in took
  a long time.** The Sign in fields were drawn only after `host.doctor` answered, and the doctor scanned the
  host key first - **14.5 s** on the user's VPS, because `ssh-keyscan` takes 13 s there. The sign-in itself
  takes about 4 s.
  * The card now shows the password and code fields **at once**, from the host's own "is not signed in"
    status line. The footer's button is **Sign in**, not a greyed-out Connect.
  * The doctor skips the scan when the probe already reached the password step. The probe checks the
    pinned key itself (`StrictHostKeyChecking=yes`), so a changed key still fails and is still scanned.
  * **Remember the password** (a checkbox, on by default) keeps it in the OS keychain once the host accepts
    it. After that, a reconnect asks only for the verification code, which is focused and is still typed
    every time. "Forget saved password" removes it, and removing the host removes it too. The password
    never leaves the keychain through any method (`host.password` answers only whether one is saved).
* **Live preview followed nothing.** It sat on `localhost:3000` while the agent built a new page on the live
  site, and that `localhost:3000` was the VPS's own port, which this computer cannot open. Now:
  * **Every finished edit is mapped to the page it serves**, using each framework's own rules: Next.js
    `app/…/page.tsx` (route groups and `[slug]` handled) and `pages/`, SvelteKit `+page.svelte`, Nuxt and
    Astro `pages/`, WordPress `page-{slug}.php`, and plain `.html`/`.php` files under their web root. The
    report's own file, `/var/www/skilleddesk.com/public_html/src/app/(public)/email-marketing-agency-usa/page.tsx`,
    is `https://skilleddesk.com/email-marketing-agency-usa`. A component or stylesheet keeps the page on
    screen and reloads it.
  * **The site is read from the edited file's path** as well as the project's (`/var/www/example.com/…`),
    so a chat whose own folder is a workspace still previews the site it is editing.
  * **The preview follows the newest page**, in every project, and says so: `Following /pricing · page.tsx`.
    An address typed by hand stays until the agent moves to another page. Live off still stops following.
  * **A live site that forbids framing is shown anyway.** skilleddesk.com sends `X-Frame-Options: DENY`
    and `frame-ancestors 'none'`, so the frame showed a "blocked" icon at any address. The preview now
    loads a site through a loopback-only proxy in the daemon (`preview.open`, one port per site) that
    takes out only the headers that forbid framing. The address bar and Open in browser keep the real
    address. A local dev server is loaded directly, as before.
  * **A dev server on a VPS opens through the signed-in connection.** `preview.forward` asks the ssh master
    to forward the port (`-O forward`, no new login), and the chip reads `host :3000` instead of an
    address that points at this computer.

## [0.14.3] — Reading a long file in pieces is not a loop

* **Found after installing 0.14.2, in a real chat on skilleddesk.com:** "Stopped a loop: `Read
  …/check-duplicate-content.js` ran 5 times in a row without anything changing." Claude was reading a long
  script in pieces - the same path with a new `offset` each time - and the loop guard compares only the
  tool's name and target. A Claude Code `Read` card now names the lines it read (`check.js · lines 101–200`),
  so five pieces are five different reads, and reading the same piece five times still stops. Only a read
  carries the range; an edit's path and a command stay bare, because those are what the policy checks.

## [0.14.2] — A file is shown while it is written, Enter starts the work, and web search is allowed

The report came with two screenshots: *"live steamming omg lvl ar bad ... monai hosse nah je live steamming
hosse"*, a request stuck on "Reading your request…", and a red `WebSearch failed` card. Each was measured
before it was changed.

* **A file is shown while the model writes it.** Measured on a dev daemon: a Claude Code turn that wrote a
  150-line file showed its `Read` card at 6.0 s and then **nothing for 31.7 s**, until the `Write` card
  arrived already finished. The model was streaming the file the whole time as tool input, and SDC ignored
  it. A new event, `ToolCallDrafting`, reports a tool call while it is still being written, about three
  times a second: the tool, the file or command as soon as that much has arrived, how many characters so far,
  and the newest lines. The stream draws it as a card with the text growing at the bottom, and the live bar
  says `Writing plan.md`. Measured in the window: from 12 s to 34 s the card showed the file line by line
  (`plan.md 2.3k chars · 8s`, `29. Quiescence search is capped at six▍`), then the real card opened.
  The SDC Agent does the same for DeepSeek, Qwen, GPT and Claude API models (both dialects); measured with
  DeepSeek: a `write_file` reported every ~300 ms from 0.8 s to 6.8 s.
* **Claude Code's `Write` and `Edit` cards show the change.** A `Write` ended as `done · 1 ln` with Claude's
  "File created successfully" as its only line. The card now carries the lines written (`done · +80`), and
  an `Edit` its removed and added lines (`done · +1 −1`), from the tool's own input.
* **Enter starts the work.** The Intent Contract card ("Reading your request…" with a disabled Confirm)
  stopped every request of eight words or more, or in Bengali, until a model had read it and you clicked.
  It is now off unless you turn it on (Settings → Language). The engine still starts its answer with an
  `Understood:` line, which now gets its own chip even when the agent goes on to use tools.
* **Claude Code may search the web.** With `-p` nobody can answer Claude's permission prompt, so every web
  search failed with "Claude requested permissions to use WebSearch, but you haven't granted it yet."
  `WebSearch` and `WebFetch` are allowed at every autonomy level: they read the web and change nothing on the
  machine. Measured: the same turn searched and answered, with `permission_denials` empty. Any other tool a
  level does not allow now says which switch allows it (Pro or Auto) instead of pointing at a prompt that
  does not exist.
* **No empty `ToolSearch` card and no checkpoint before every web search.** Claude Code loads a tool's
  definition with `ToolSearch` first; it was drawn as a "run" card (`done · 0 ln`) and took a checkpoint.
* The narration's caret no longer blinks after the words have stopped, and the live bar's token count and
  pace include what a file being written adds.

## [0.14.1] — Claude Code's tool cards show what ran, and the loop guard stops only real loops

* **A Claude Code tool card has its command again.** With the partial stream, a tool call's first line
  always carries an empty `input: {}`, and the command arrives afterwards in pieces. SDC read the card from
  that first line, so every Bash card had only its name. The card now opens from Claude's finished
  `assistant` line, which has the whole input. Protected paths and denied commands are checked against the
  real target again for Claude Code turns.
* **No false "Stopped a loop".** Five different Bash commands with no target looked like one command run
  five times, so a working turn (the same prompt Claude Code finishes on its own) was stopped. Calls without
  a target never count as a repeat now, and a call named twice opens one card and counts once. Measured:
  six different Bash calls in one Claude Code turn, each card with its command and output, none stopped.
* **A card shows what the tool printed.** A Claude Code result was only a `done · 10 ln` pill over an empty
  box. The first 40 lines are now the card's output, with a "… N more lines" line, and a failed result is
  shown in red. A card with nothing to show has no empty box under it.
* **The stop reason is said once.** It was printed as the turn's ending and again in red on the footer.
  The footer now says only `Stopped: loop guard` or `Stopped: budget`, with the whole sentence on hover.

## [0.14.0] — The agent uses a browser, Gemini remembers, and effort is a choice

This release closes the gaps 0.13 listed as open.

* **`browser` tool.** The agent drives a real headless Chrome or Edge over the DevTools protocol. It can
  open a page and read its text, plus a numbered list of what can be clicked or filled. It can click, type
  into fields, and press keys. A model that can see images can also take a screenshot. The browser stays
  open between the agent's steps in a chat. The person's own dev server and local files are opened without
  asking; a public site is asked about first, because clicks there are real. This was measured with
  DeepSeek: it wrote a form, typed a name, clicked Greet, and read "Hello, Rongdhonu!".
* **Screenshots on every platform.** `screenshot` goes through the same DevTools connection. On the macOS
  CI runner, the old `--screenshot` flag wrote nothing. The browser test now runs on Windows, macOS and
  Linux (`--no-sandbox` on Linux).
* **Gemini resumes its own conversation.** SDC starts each chat's Gemini session under its own id
  (`--session-id`) and continues it with `--resume <id>`. The id form is resolved in Gemini CLI's source,
  even though its help only mentions "latest" and an index.
* **Effort.** The model menu has a new row: Auto · Low · Medium · High · Max. It becomes Claude Code's
  `--effort` (measured with a real call), Codex's `model_reasoning_effort`, and `reasoning_effort` for
  OpenAI's reasoning models. Other models are sent nothing, because an unknown field can make a provider
  refuse the whole request.
* **`apply_patch` for GPT and Codex models.** This is Codex's own edit format: one patch that can add,
  update, move and delete several files. Every hunk is checked against its file before anything is written,
  so a patch written against an old version changes nothing and names the hunk that did not fit. Each file
  still goes through the policy, the permission question, the checkpoint and a diff card.
* **The owner's VPS address and username are removed from the whole git history,** not only from the
  current files.

## [0.13.2] — Measured on the VPS: an attached image reaches Claude Code

On the owner's VPS, with a 2FA sign-in, 0.13's host paths were run for real, and 6 of 6 checks now pass.
The SDC Agent (DeepSeek) ran grep and glob on the host and fixed the bug there. It also started a server
with `start_process`, reached it with curl and stopped it. The project's MCP server ran on the VPS and
answered `HELLO from vmi2978466`. Claude Code on the VPS resumed its own conversation on the next turn.

One check failed first, and that failure is this release's fix. An image attached to a Claude Code turn is
saved in SDC's attachments folder, which is outside the project. Claude Code answered "I need permission to
read the image file", and in `-p` mode it can never be given that permission. The folder is now passed with
`--add-dir`. This was measured on the VPS (a red swatch was answered "Red") and on this machine (a green
swatch was answered "Green").

## [0.13.1] — 0.13.0 on every platform

0.13.0 built only for Windows. On macOS and Linux, two tests failed that never run on Windows:
* A unix-only keychain test wrote a key through `set` and then checked the key file on disk. Since 0.13.0,
  a test build keeps secrets in memory, so no file was written. The test now writes the file itself.
* macOS's Chrome, running headless on the CI runner, did not render a `file:///` page for the `screenshot`
  test. The render is measured on Windows. On other platforms the tool now tells the model why it failed.

## [0.13.0] — Everything Claude Code and Codex do that SDC did not, and a cleaner live stream

From the report: *"claude code ar thakaw better hoi"*. Every item below was run against real engines (Claude
Code, DeepSeek, Qwen) through `_verify/live-013.mjs`, and 16 of its 16 checks passed.

### Memory and context
* **Claude Code and Codex remember their own conversation.** Before this, each turn restarted the CLI and
  pasted the whole chat back in as plain text. The chat now resumes the CLI's own session (`claude
  --resume`, `codex exec resume`) on the same machine and folder, and sends only the turns it has not seen.
  In the measured case, turn 2 recalled a word from turn 1 and sent 16 tokens. If the old session is gone,
  the turn restarts with the full history instead of failing.
* **A long chat no longer outgrows the model.** History is fitted to the model's context window: the newest
  turns are kept whole and older ones are summarised. `/compact` asks the chat's own model for a summary to
  continue from. During a long agent turn, old tool output is folded away. A meter in the composer shows how
  full the context is once that matters.
* **Memory.** The agent's `remember` tool, `/remember <fact>`, `/memory` (an editor for project and global
  memory), and a global memory that every project reads.

### The agent's tools (API and local models)
`grep` (regular expression), `glob`, `read_file` by line range, `web_search` and `web_fetch` (public pages
only, with every redirect re-checked), `start_process` / `process_output` / `stop_process` for dev servers,
`ask_user` (a question card with options), `task` (read-only sub-agents; several run in parallel), and
`view_image` / `screenshot` for models that can see images (a headless Edge or Chrome renders the page).
Images can be pasted or attached to a message. A project's `.sdc/mcp.json` servers also run on a VPS, over
the same ssh connection. Skills (`SKILL.md` in `.sdc/`, `.claude/`, `.codex/` or `.agents/skills`) and
`[hooks] after_edit` in `.sdc/policy.toml` work the way they do in Claude Code and Codex.

### Finishing the work
* The default step limit rose from 25 to 60, so a task can finish in one turn.
* If the agent says it is done while its plan still has open steps, it is reminded once.
* If it changed code and ran nothing afterwards, it is told the project's checks and asked to run them,
  unless the person said not to. That case was measured: *"test chalanor dorkar nai"* ("no need to run
  tests") was respected. If a check fails in code the agent did not touch, it reports the failure instead
  of changing unrelated code.
* The answer stays in the person's language. DeepSeek had drifted into Chinese.

### The composer and the stream
* **Composer.** `/` opens the command list (SDC's own commands, plus the project's `.sdc/commands` and
  `.claude/commands`). `@` finds files in the project. The controls sit on one row at every width. The
  Chat | Agent switch is gone: every turn is an agent turn, which answers a question without using tools,
  the way Claude Code works.
* **Stream.** Consecutive reads fold into one line (`Explored · Grep add · Read math.js, test.js`). A short
  edit shows its diff inline, with indentation kept. A sub-agent's card lists what it read, and a question
  stays in the turn with its answer. Every finished turn ends with `Changed 2 files +13 −2`. Claude Code's
  TodoWrite list and Codex's todo list both appear as the plan card.
* **Notifications.** The Notifications settings, and the "Test sound" button, did nothing before this
  release. A turn that finishes, or needs you, while SDC is not in front now shows a desktop notification.
  Sound cues play when they are turned on.

### Fixed along the way
* On Windows, a command with double quotes in it (`node -e "…"`, `git commit -m "…"`) reached `cmd` mangled.
  Commands now go through `cmd /S /C` with the line passed through as written.
* Qwen-VL sends each tool call's arguments cumulatively (the whole text so far, every chunk). They were
  appended, which produced broken JSON.
* A message sent the moment a turn ended (a queued prompt) could miss that turn's answer. The answer is now
  stored before the turn is announced as finished.
* Nothing that erases data can reach the OS keychain in a test build any more. A test did exactly that on
  the developer's machine during this work.

### Your data
* An installer never carried anyone's data. Chats, hosts and settings are in `%APPDATA%\sdc`, and keys are
  in the OS keychain. Before this release, though, uninstalling and installing again brought all of it back.
  The uninstaller's "Delete the application data" box now removes SDC's folder and keys, and **Settings →
  Safety → Erase all SDC data** does the same from inside the app. An update never erases anything.
* The owner's real VPS address and username have been removed from the test files in this public
  repository.

## [0.12.8] — An update to a closed chat is refused, not silently accepted

Found while checking 0.12.7 installed over the running app. To restore example-shop.com's chat, a
`session.update` bound the folder to `n49400`, a chat closed hours earlier. The real chat was `n49430`. The
daemon accepted the update and announced it, so the fix looked done while the chat on screen still had no
folder. `session.update` now refuses a chat that was closed or never opened, and names it. example-shop.com's
chat is bound to `/var/www/example-shop.com` again.

## [0.12.7] — A project cannot be removed by one slip, and one Claude sign-in measured on a real VPS

* **Removing a project now takes two deliberate clicks.** In 0.12.6 an ✕ appeared right beside a project's
  `+` on hover, and one click removed `example-shop.com` from the list. Its chat and conversation were kept, but
  the chat lost its folder; both were put back by hand. Hovering a project now shows `+` and `⋯` only. `⋯`
  opens a small panel showing the folder's full path and "Remove from list". The first click on that button
  only arms it ("Click again - 1 chat is kept"), and it disarms itself after four seconds.
* **One Claude sign-in for every host, verified.** On the owner's VPS, whose own `claude` was signed out
  (`loggedIn: false`), a claude_code turn answered `forwarded-ok` using this PC's sign-in. Afterwards no
  secrets file was left in `~/.sdc/run`, and the VPS's own `~/.claude/.credentials.json` was unchanged.

## [0.12.6] — A new sidebar, and a composer that holds its own controls

From the report: *"side bar ar style and sytem ta valo lagse nah … new design"* and *"chat box ar vitore
ARO better quality kore"*.

* **The sidebar is redesigned.** From top to bottom:
  * New chat and the search.
  * **Machines**: this computer and each VPS as a row, showing status and chat count, with a spinner while
    one of its chats is working. "Add host" sits below them.
  * **Projects** for the machine you selected. Each project has a coloured badge, and the one worked on
    most recently comes first. Opening a project shows its chats and a New chat for it; hovering it offers
    New chat and Close.
  * **Chats** outside any folder.
  * **Files**, as before.

  Only one machine is shown at a time, instead of one long tree of every host, folder and chat. A search
  still looks through every machine at once and groups the results by machine. New chat opens in the
  machine and project you are looking at.
* **The composer holds its own controls.** Chat or Agent, the model and the folder moved from the row
  above the box into the box itself. The row inside the box is now: attach, image, voice | Chat or Agent |
  model | folder | a round Send button. While a turn is running, the box offers to add your message to
  that turn, and shows Stop beside Send.

## [0.12.5] — A live stream in the order it happens, chats per project, and one sign-in for every host

From the report: *"every step every process jano dakha jai AI ki korse ki think korse"*, *"project base
alada chat"*, *"local a file select korar manual kono option nai"*, *"new chat a je bisoye likbo … rename"*
and *"CLI and API gula akbar connect korlei jano local and vps sob jaigai kaj kore"*.

* **The turn is drawn in the order it happened.** Before this, a turn drew one thinking box at the top, all
  the tool cards under it, and every word the agent said joined into one answer at the end. While the agent
  worked, its newest thought was in a box that had scrolled out of view, and a line like "Now I understand
  the project, let me…" only appeared once the turn was over. Each thought, remark, tool call and checkpoint
  now sits where it happened (`TurnView.timeline`, rebuilt from the event log, so older chats read this way
  too). The thought in progress stays open and streams. A finished thought folds to "Thought for 2.4s" with
  its first line still showing. The words the turn ends on keep the Answer card.
* **"Deciding the next step…"** fills the gap between two actions, with its own timer. That pause used to
  look like a stalled turn.
* **A sticky live bar above the input** shows what is happening now (thinking, running a named tool,
  writing, deciding) and for how long. It also shows the plan step in progress, the elapsed time, tokens,
  tok/s and tool calls, plus "Jump to now" when you have scrolled up. It replaces the stats line that
  scrolled away.
* **Chats per project.** In the sidebar, each host now lists its folders, and each folder lists its own
  chats and has a `+` for a new chat in that folder. `+ New chat` reuses an empty chat only when it is in the
  same folder, so it no longer takes over another project's chat.
* **Open a folder from the sidebar.** Each host has "Open a folder…". On this machine it uses the system's
  own folder picker, and on a VPS it uses the host's folder browser.
* **Chats name themselves.** A chat that still has its default name (`New chat` or its folder's name) is
  renamed from the first message sent in it. A name you typed yourself is never changed.
* **One Claude sign-in for every host.** A Claude Code turn on a VPS now uses this PC's Claude sign-in. The
  token goes over ssh's stdin into a `umask 077` file, which the turn reads and deletes before `claude`
  starts. It is never in a command line, and the host's own `~/.claude` is not touched. When the token is
  close to expiring, a short local `claude` call renews it first. Codex and Gemini are not covered yet. API
  providers already worked everywhere, because their calls are made from this PC.
* **One style across models.** Each turn in a folder now starts with a short continuity brief. It carries
  the project's formatting rules (`.editorconfig`, Prettier, ESLint, Biome, rustfmt, ruff, and others), its
  stack from the manifest (React, TypeScript, Laravel, WordPress, and so on), and the project's rules file
  when the engine's CLI would not read it by itself. When the earlier turns in the chat were written by a
  different model, the brief also includes a hand-over: who wrote them, the files they changed, and an
  instruction to read those files first and match their layout, naming and formatting instead of restyling
  them (`continuity.rs`).
* **Messages sent while a turn is running now join it.** An agent turn (API or Ollama) reads anything you
  send between steps, after a tool call or just before it would finish, and adjusts. The message appears
  in the timeline where the agent received it, labelled "You, while it worked". A CLI turn cannot take a
  message mid-run, so it still queues the message as the next turn, as before (`engine.steer`,
  `TurnSteered`).
* **Live preview.** The Preview tab has a Live switch, which is on by default:
  * The page reloads after each change the agent finishes.
  * A dev server address in a command's output (`Local: http://localhost:5173/`) is picked up and opened.
  * Addresses the chat found, and a VPS project's own domain, appear as one-click chips.
  * The tab shows which change caused the last reload.
  * Turn the switch off to reload by hand.

## [0.12.4] — Alibaba Cloud: its latest models, more than one in use, and errors that say which key

From a report with two screenshots. A **claude_code** chat on the VPS failed with `OAuth session expired`,
and a **qwen3-max** chat failed with Alibaba's `Invalid API-key provided (401)`. Both cards said "This
failure has no rule yet".

* **"Qwen (Alibaba)" is now "Alibaba Cloud".** The provider id stays `qwen`, so a saved key and Base URL
  keep working.
* **The latest models.** The bundled list now has Qwen3.8 Max and Flash, Qwen3.7 Max, Plus and Flash,
  Qwen3.6 Plus, Qwen3 Coder Next, Kimi K3, Kimi K2.7 Code, GLM-5.3 and GLM-5.3 Prime. These rows were taken
  from Alibaba's own `/models` on 2026-09-28. The live list was already refreshed when the daemon starts,
  every 12 hours and when a key is saved. It had fallen back to the old bundle only because the saved key was
  refused. With a working key it lists all 172 ids, including DeepSeek V4.1 Flash. DeepSeek V4 ids are left
  to the live list, because DeepSeek's own API uses the same ids.
* **More than one model in use.** `Use` in Connect now adds a model instead of replacing the one that was in
  use, and `Remove` takes one out. A provider with models in use shows exactly those in the chat's menu, and
  the rest behind `Older versions…`. `models.select` gains `remove`, and `models.list` and `models.select`
  answer `inUse`.
* **Speech, image, translation and OCR models are left out** of the Connect list, as they already were in the
  chat menu. Alibaba lists about 170 ids, and most of them cannot run a coding turn.
* **Two new error rules.** `api-key-rejected` handles a provider that refuses the saved key: paste a fresh one
  on the card. `engine-signed-out` handles an expired CLI sign-in. A VPS keeps its own `claude` login in its
  own `~/.claude`, apart from this PC's, so the fix is `/login` on that machine.

## [0.12.3] — Qwen (Alibaba): the key check goes where the key lives

Found by installing 0.12.2 from GitHub over the running app. A **qwen3-max** chat in it had failed with
Alibaba's `Incorrect API key provided (401)`.

* **The key check ignored the saved Base URL.** `provider.test` always asked the built-in
  `dashscope-intl` host, even with a workspace URL saved. It now asks the saved one, the same as a chat turn
  and the model list do.
* **A pasted key keeps no whitespace.** A key is cleaned on save and on Test. Keys read from the Windows and
  macOS keychains are trimmed too, as the file store's keys always were.
* **Alibaba's 401 says what to do.** Alibaba gives the same sentence for a wrong key and for a key made in
  another region or workspace, so SDC adds: create the key under Model Studio → API Keys in the workspace
  whose Base URL is set, then paste it again.

Measured: the key stored on this machine was refused (401) by `dashscope-intl` (Singapore), `dashscope`
(Beijing), `dashscope-us` and the workspace host `ws-….ap-southeast-1.maas.aliyuncs.com`. The key itself is
the problem, not the address. The workspace Base URL stays saved, so a new key from that workspace works as
soon as it is pasted.

## [0.12.2] — an accessibility audit that measures a settled screen

0.12.1's Windows and macOS runs still failed the axe-core audit, each time on a *different* piece of text in the
first-run dialog. The dialog fades in over 220 ms, and on a slow runner axe measured the text half-transparent.
The audit (`app/scripts/a11y-bundle.mjs`) now waits for the screen to settle: every finite animation finishes
before it measures, while a spinner is not waited for. It also names the dialog it audited (`open dialogs:
Welcome to SDC`) and takes `SDC_A11Y_SCHEME=light|dark`, so a green run on one machine says which screen and
which theme it covered. The dialog's small grey text is secondary text now, for margin in both themes.

## [0.12.1] — 0.12.0, with the two release checks it failed

0.12.0's release run stopped before the Windows and macOS installers. This is the same release with those two
checks fixed:

* **Accessibility (axe-core, on the built window).** On the first-run card, the selected goal's help text was
  muted grey on the accent tint, about 4.0:1 in the dark theme where 4.5:1 is the floor. It is secondary text
  now (about 6.3:1). The same fix applies to the chosen row in Agency → deploy history.
* **Secret scan (gitleaks).** The scanner's own tests held a made-up AWS key id as a literal. It is built at
  run time now, and `.gitleaksignore` lists the two fingerprints of the commit that held it.

## [0.12.0] — the Trust Kernel: every change proven, priced, reversible and on the record

The plan: [`sdc/docs/MASTER-PLAN-v3-TRUST-KERNEL.md`](sdc/docs/MASTER-PLAN-v3-TRUST-KERNEL.md). This release
builds all of its phases in one go. Tested live against an isolated daemon with DeepSeek: an agent turn wrote
`math.js` with a checkpoint taken first, cost was priced per turn, Verify came back **PASS** (the check ran,
secret/SAST scans ran, a second model reviewed it) and Trust went from 70 to 100. A Banglish request was read
and compiled for Claude Code, the kill switch stopped a streaming turn and took a checkpoint, the audit chain
verified intact, and a Bengali Proof Pack was written.

### Added: Trust Kernel (`sdcd/src/trust`)

* **Audit ledger.** Every event is also written to a hash chain. `audit.verify` finds a row that was changed,
  and `sdcd audit` does the same from the command line.
* **Policy as code** (`.sdc/policy.toml`, editable in **Policy**). Sets protected paths (`.env`,
  `wp-config.php`, keys, backups), denied commands, commands that always need a yes, blast-radius limits,
  privacy (local models only) and budgets. The agent's tools enforce it. A turn that reaches a protected path,
  denied command or blast-radius limit is stopped and recorded.
* **Secret and SAST scanning** of the added lines on every Verify, plus the project's own dependency audit
  (`npm`/`pnpm`/`yarn`/`pip-audit`/`cargo audit`) when it is installed. Findings are redacted.
* **Cost governor.** Engines report token usage, and every turn has an estimate before it starts and a settled
  price after. Session, daily and monthly budgets stop a turn at the limit. A runaway guard stops a loop that
  burns tokens without changing anything. The router suggests a cheaper model for simple work. A **Cost** meter
  sits in the status bar, and **Cost Center** gives the breakdown.
* **Kill switch** (top bar, `Ctrl+Shift+.`). Stops every turn, command and deploy, and checkpoints each folder
  as it was.
* **Trust score** on every turn: rollback point, verified or not, protected paths, blast radius, cost. An
  **Ops score** per site.
* **Proof Pack** (`Ctrl+Shift+P`). JSON and a self-contained HTML page in Bengali, Hindi, Arabic, Spanish or
  English: what was asked, what changed, checks, scans, review, cost and the ledger rows.

### Added: Universal Intent Engine (`sdcd/src/intent`)

* Offline language detection covers every script plus Banglish, Hinglish, Arabizi, Roman Urdu and Taglish,
  and the dialects (Sylheti, Chittagonian, Noakhali, Bhojpuri, Egyptian, Gulf, Levantine, Maghrebi, Swiss
  German).
* **Task Spec.** A request is read into goal, scope, out-of-scope and acceptance criteria, with a
  back-translation in the person's own language. The **Understood** card asks "is this what you meant?", and
  corrections go into a per-project glossary.
* **Prompt compiler.** One spec is compiled per engine (Claude Code, Codex, Gemini, API agent), with the
  policy's protected paths written in.
* **Voice input**: whisper.cpp locally, or Groq/OpenAI when a key is stored.
* **Long-task memory**: a plan survives a restart, and later turns see it.

### Added: Verify
Verdicts are now **PASS / FAIL / NO_CHECKS / UNPROVEN**. `pass` means proven, so UNPROVEN is not a pass. A
condition about tests, build, lint or types that a check SDC ran has measured is decided by that measurement,
not by a reviewer's reading of the diff. Found live: the reviewer saw only `math.js` and said
"npm test cannot pass" right after it passed. A skipped review is marked as skipped, never as passed.

### Fixed: Time Machine (TM-1 to TM-8)
* A restore is journaled and survives a crash halfway through (`recover()` at start-up).
* A rewind also rewinds the conversation, so the next turn does not see the undone work.
* **Per-file restore** with a diff for each file, locally and on a VPS.
* Checkpoints carry a label, the turn that made them, and an *irreversible* mark when a turn did something no
  file restore can undo (a DB command, a deploy).
* Rewinding while a turn is running is refused with a clear sentence, not a half-restore.

### Added: Agency (`sdcd/src/ops`, **Agency** `Ctrl+Shift+G`)
* **Sites** and a **Safe Deploy** pipeline: preflight, then a mandatory backup (files and DB), steps (with the
  deny list), restart, health check, and automatic rollback on failure. Restoring a DB needs the word
  `RESTORE` typed.
* **Health Watch**: a background checker with alerts.
* **Night Guardian**: at night it only rolls back, and a fix waits for a person.
* **Takeover X-ray**: a read-only scan of an unknown server (web servers, WordPress installs, databases,
  Docker, cron).
* **Shadow DB rehearsal** runs a migration against a copy first.
* **Staging and a client approval page**: a small PHP receiver with a token.
* **Playbooks** and **team roles** (owner/dev/viewer), enforced by the daemon on every call.
* **Headless**: `sdcd run | verify | kill | audit` for CI and cron.
* **MCP tools** in the API agent, **crash reports** kept locally, a **status share** for a phone, and a
  **low-bandwidth mode**.

### Added: global UI
* The interface is in **10 languages**: English, বাংলা, हिन्दी, العربية, اردو, Español, Português, Français,
  Bahasa Indonesia and 中文. Arabic and Urdu lay out right to left, and code stays left to right.
* The prompt no longer sends mid-composition in Bengali, Chinese or Japanese input methods.
* Onboarding for a first run.

### Added: provider **Qwen (Alibaba)**
Alibaba Cloud Model Studio through its OpenAI-compatible endpoint, with the streamed reasoning shown as
thinking. Models: Qwen3 Max, Qwen3 Coder Plus, Qwen Plus/Flash, Kimi K3, Kimi K2 Thinking, DeepSeek V3.2,
DeepSeek R1, GLM-4.6 and MiniMax-M2. The Connect dialog takes the workspace's own **Base URL**
(`https://<workspace>.<region>.maas.aliyuncs.com/compatible-mode/v1`), and it is saved per provider. Wan and
HappyHorse are image/video models: they are recognised and kept out of the chat model list, because a chat
turn cannot run them.

## [0.11.9] — measured on the real VPS: a command line that reached it, faster commands, a switch that stays

This release was checked against the report's own VPS (password + verification code, 217 ms away): the
sign-in took **8.1 s** end to end, a command silent for **25 s** and one silent for **60 s** both finished
with exit 0 (before 0.11.7 they were cut at 15 s), and the Terminal's shell opened a real prompt there.

### Fixed — a command line sent to a VPS as Windows `cmd /C`

`shell.run { line }` wrapped the line in the *local* platform's shell, so on Windows a VPS was sent
`cmd /C <line>` and answered `bash: cmd: command not found`. Every line typed into the Terminal's
**Commands** for a VPS chat failed. On a host the host's shell runs it now (`sh -c`).

### Changed — quicker commands through the signed-in connection

Each command asked `ssh -O check` first (~65 ms). A live answer is now trusted for 2 s, so an agent's
burst of commands skips it; the watcher still checks afresh every 5 s. Measured on the VPS: a quick
command went from 550–740 ms to about 460 ms - the rest is the network (2.3 round trips of 217 ms).

### Added / changed

* **Settings → General → "Understand my messages in any language"** (on by default). Off sends the message
  exactly as typed.
* **Settings remember their switches.** They were kept in the dialog only and reset every time it opened.
* The **Understood** card speaks to you - "আপনি চান…", not "ব্যবহারকারী চান…".

## [0.11.8] — SDC reads you the way you write, and a stream that flows

The report: *"ami jevabe tmake sms kortasi ai vabe sms korle jano SDC bujte pare and promt make kore
automatic"* - write the way you text, in any language, and have SDC turn it into a proper request - and a
live stream better than the chat apps.

### Added — reading a message the way it was meant (`sdcd/src/understand.rs`)

* Every message's language is detected: **Banglish** (Bengali in English letters), Bengali script,
  Hinglish, Hindi, Arabic script, others. Scripts are counted; romanized Bengali and Hindi are recognised
  by their common words - the report's own messages are the tests.
* The engine receives a short brief in front of the message: read past the spelling, find **every**
  request in it, restate it, do each part, answer in the person's language, keep code and commands as they
  are, and ask before a costly guess. The message itself is sent **verbatim**, and only the message is
  stored and replayed.
* The answer opens with one line - `Understood: …` in the person's language - drawn as its own card, so
  you see at a glance whether SDC got it right before reading any work. Measured end to end with a
  Banglish request: the card and the whole answer came back in Bengali, the code untouched.
* The message carries a chip: *Read as Banglish · answering in বাংলা*. Short English lines and `/commands`
  go through untouched; `engine.start { understand: false }` turns it off.

### Changed — the live stream

* **Even reveal.** Text is drawn a few characters per frame, always a fixed share of what is behind, so it
  flows at the engine's pace instead of jumping in 50 ms lumps - and it never stops inside a character: a
  Bengali conjunct (ক্ষ), a vowel sign or an emoji arrives whole. Reduced-motion settings get the text at
  once.
* **Only the last paragraph is re-drawn.** Markdown is parsed per paragraph and memoised, so a long answer
  no longer re-parses from the top on every frame.
* **What it is doing now** leads the live line: `Run · git status`, `Thinking`, `Writing`.

### Fixed

* **Errors that blamed the wrong thing.** `Permission denied (keyboard-interactive)` was explained as *"the
  path is not writable"*; it is now *Not signed in to the host*, with the way back. `env: 'gemini': No
  such file or directory` now says the CLI is not installed on the machine the chat runs on. A quoted line
  no longer shows a stray double backtick.
* **Reconnect opens the sign-in card at once** for a host that only needs its password and code, instead
  of probing for two seconds first.
* **The Terminal on a signed-out host** offers *Sign in*, and reopens the shell by itself once you are in.
* **The status bar's daemon version** is the running daemon's (it showed `sdcd 0.11.6` next to a 0.11.7
  daemon after the update).

## [0.11.7] — the 15-second cut, a sign-in without the 13-second wait, and a real terminal

The report: a VPS chat dropped every so often (*"maje maje onk druto disconnect hoye jasse"*), the sign-in
card after a drop behaved oddly, signing in was slow, the chat stream should follow the text, and there was
no terminal. Every item below was traced in the daemon's own event log first.

### Fixed — `Connection to … closed by remote host.` fifteen seconds into a quiet command

Every one of those lines in the log came **14.6–15.0 s** after its command started — one
`ServerAliveInterval`. Commands for a signed-in host run as `ssh -O proxy` clients of the one master
connection, and each client carried `ServerAliveInterval=15`. After 15 s without output it sent a
`keepalive@openssh.com` request to the master — and OpenSSH's mux proxy relays only `tcpip-forward`
global requests (`channels.c`, `channel_proxy_downstream`: `unsupported request`), so it closed that
client. The command was cut, its channel orphaned on the master, and once the master went down with it:
every later command fell back to the key and got `Permission denied (keyboard-interactive)`.

* A call that goes through the signed-in connection now sends **no keepalive of its own**
  (`ServerAliveInterval=0`); the master keeps the real connection alive.
* The master itself tolerates two minutes of network silence (`ServerAliveCountMax=8`) instead of 45 s.

### Fixed — a lost sign-in is seen in seconds, and the agent stops retrying

* The watcher checks each signed-in master **locally every 5 s** (`-O check`, no network) instead of
  noticing up to 45 s later with a remote probe. A master left by the daemon an update replaced is
  adopted and watched the same way.
* When the connection is gone, an agent's `run` is a tool error that says so and asks the person to sign
  in again — the report's agent tried four more commands, 3.7 s each, all refused.

### Fixed — signing in took 13 seconds before it started

A sign-in to a host whose key is already pinned now goes **straight to the sign-in**. The 13 s were the
host-key scan (`ssh-keyscan` fails that host's key exchange, then a full handshake). The sign-in checks
the same pin itself with `StrictHostKeyChecking=yes`, so a changed key still stops it before a password
is offered — and it gets the pin's own sentence. The 13 s also ate into a 30-second authenticator code.

### Fixed — the sign-in card after a drop

The card stopped its spinner and re-asked the doctor as soon as the daemon *accepted* the sign-in, while
the sign-in was still running — so a stale "Sign in" form sat under a "Waiting for the host to answer…"
header. It now follows the host's own status: "Signing in…" until it lands, then closes with *Signed in*,
or stays with the host's sentence. The header says *Signed out — sign in to continue* over the form, the
password field is focused, and Enter moves to the code and then signs in.

### Changed — the chat stream

* Streamed text is pushed every **50 ms** instead of one event per token. One turn in the report's log was
  14 000 `TurnDelta`s — each written to the database, sent over the bridge and re-rendered. It reads just
  as live, at a fraction of the work.
* The view follows **everything** that grows while you are at the bottom — tool output, plans, permission
  cards, code blocks — not only the answer's length. A new message takes you back to the bottom, and when
  you have scrolled up a **Jump to latest** chip appears.

### Added — a real terminal

The Terminal tab opens on a live **Shell** (xterm.js), next to the recorded **Commands** log. On a VPS it
is `ssh -tt` through the **same signed-in connection** — a real prompt, colours, `cd`, `top`, `nano`,
Ctrl+C, and nothing asked again; on this computer it is the platform's shell. `Ctrl+\`` (or *Open terminal*
in the palette) opens it. `pty.output` takes a byte cursor (`since` → `data`, `next`) for this.

## [0.11.6] — the real reason the providers vanished, and an update that replaces the daemon

0.11.5 was installed on the machine the report came from and checked in its window. It came up
`0 providers · 0 chats · 0 hosts` - the report, exactly - with the daemon answering every one of those
lists in under seven seconds from a terminal. Two defects were behind it, and neither was the one 0.11.5
fixed.

### Fixed — thirty-five thousand messages in front of every call

On its first connect the desktop bridge asked the daemon for the **whole event log** and forwarded it to
the window one `emit` at a time. The page already asks for that history itself (`lib/sdcp.ts`, one
`event.list` folded in a loop), so this was a second copy - and on this machine's log of 35 249 events it
was 35 249 IPC messages queued ahead of everything else. `sdcp_status`, a command that touches no socket
at all, did not answer for more than twelve minutes; the window sat empty the whole time. The longer SDC
is used, the longer the log and the longer the wait, which is why it looked like providers had been
*removed* rather than slow to load.

* **A first connect only subscribes** (`event.subscribe`); the page's own catch-up is the history. A
  reconnect while the window stays open still asks for what it missed since the last event it forwarded.
* Measured after the fix on the same machine and the same log: the window shows `4 providers · 2 chats ·
  2 hosts` within fifteen seconds of launch, and `sdcp_status` answers at once.

### Fixed — an update that left the old daemon behind

Installing 0.11.5 while SDC was open left `sdcd.exe` at 0.11.4 (`sdcd --version`, after the installer
finished): Windows locks a running program's file, and the installer skipped it. The window then talked to
the previous release's daemon. The NSIS installer now stops `sdc.exe` and `sdcd.exe` before it copies
(`src-tauri/windows/hooks.nsh`) - verified by installing 0.11.6 over a running 0.11.5: `sdcd 0.11.6`.

### Verified in the installed window

* The banner's `Reconnect` on the 2FA VPS opens the host's card with the password and verification-code
  fields (0.11.5's `watchHosts`), after the doctor names `Sign in`.

## [0.11.5] — the window asks again: providers after an update, a sign-in after a drop, a site by its name

Three reports, and all three came down to a window that asked once and then stopped asking.

### Fixed — "all my CLI and API providers were removed"

Nothing had been removed: asked directly, the daemon still answered Claude, OpenAI and Gemini signed in
and a DeepSeek key saved. The window had asked for them **once**. `connectDaemon()` ran when the window
mounted, and the first launch after an update is exactly when the daemon is still being replaced - the
call failed, and nothing asked again. The heartbeat saw the daemon come up and said `back online`, while
the Provider Hub, the model menu and the sidebar stayed empty until the app was relaunched.

* **The heartbeat loads the lists** whenever the window does not have them yet, and again whenever the
  daemon comes back (a new daemon is a new answer). The retries are quiet - the banner already says the
  daemon is away - and only one load runs at a time.
* **A socket that failed is a socket that is gone.** The desktop bridge only cleared its `connected`
  flag on end-of-file, so a daemon replaced under the window left a dead stream behind and every later
  call failed with `os error 10054`. A write or read error now clears it, and the next call connects
  again (which also starts a daemon that is not running).

### Fixed — "SSH suddenly drops, and nothing asks me to reconnect"

The VPS in the report signs in with a password and a verification code, and SDC holds that sign-in
open as one master connection. When the master goes - an update replaced the daemon, the laptop slept,
the network dropped - nothing said so: the log shows the row `connected` from 12:26 to 16:21 (seq 22483
→ 35235) with no connection behind it, and `Reconnect` measured, said `offline`, and stopped there.

* **The daemon notices** (`ssh::watch`, new). Every 45 s each VPS row that says `connected` is probed
  again; a failure is confirmed by a second probe three seconds later, and only then is the row written
  `offline` and a `HostStatus` pushed. A verdict that did not change pushes nothing.
* **The window asks for the sign-in** (`watchHosts`, new). A VPS that turns `offline` from `connected`
  or `connecting` - a drop, or a `Reconnect` that measured it down - opens that host's card, which is
  where the password and the code are typed, with `Lost the connection to … - sign in again to
  reconnect`. It is armed after the boot replay, so yesterday's drop does not open a dialog today, and it
  never opens over Settings, Connect or itself.

### Fixed — "I said I want example-shop.com's files, and nothing happened"

The domain matchers only knew what had been saved: a project by its name, or a host by the domain in
its address. A VPS added by its **IP** has no domain to match, so a prompt naming one of its sites ran
in whatever chat was open and the engine - any engine - was never given the site's folder.

* **`domainMentionedIn`** (new) finds a domain in the words when neither saved matcher did - and never
  takes `index.php`, `app.tsx`, `0.11.5` or an IP address for one.
* **The machines are asked** (`project.locate`): the VPS the prompt was typed on first, then every other
  connected VPS. The first whose web server config - or a conventional web folder - serves the domain
  wins. A folderless chat is bound in place; a chat already working on another folder is left alone and
  the site gets a chat of its own, so two sites are two chats that can both run.
* **Saved under the domain.** The project is called `example-shop.com`, not `public_html`, so the sidebar
  lists it by the name the person used and the next prompt that names it is routed without asking a
  machine again.

### Changed — the Gemini card says what Google says

Measured again on 2026-09-26 with Gemini CLI 0.60.0 and a finished sign-in: every turn still ends
`IneligibleTierError: This client is no longer supported for Gemini Code Assist for individuals`. That
is Google's decision about the CLI and personal accounts, and no sign-in in SDC can change it. The
card used to say only `Connected`; it now says the CLI is not served to personal accounts and names the
route that works - **Google Gemini API** with a key, the same models over `generativelanguage.googleapis.com`.

### Tests

* `a_connected_host_that_went_away_is_reported_once` (`sdcd/src/ssh/watch.rs`) - a `connected` row at
  `127.0.0.1:1` turns `offline` once, keeps its platform line, and an `offline` row is never probed.
* `store/reconnect.test.ts` (new) - the heartbeat that reaches the daemon after a failed launch loads the
  providers; a VPS that turns `offline` opens its sign-in card, including after a `Reconnect`; the card
  does not open over Settings.
* `store/domain.test.ts` (new) - the domain matcher's yes and no cases, a folderless chat bound in place,
  a working chat left alone with a new chat for the site, and a domain no machine serves.

## [0.11.4] — the 0.11.3 tree, re-cut

No behaviour changed between `20cdef9` (v0.11.3) and this tag: the five version files moved, and nothing
else did. The entry is here so the number is not a mystery in the record - `0.11.4` exists to give the
0.11.3 source a downloadable build whose number can be read back off a fresh install.

### Changed

* **One number, five files.** `sdc/package.json`, `sdc/app/package.json`, `sdc/app/src-tauri/tauri.conf.json`,
  `sdc/sdcd/Cargo.toml` and `sdc/app/src-tauri/Cargo.toml` all say `0.11.4`, so the status bar's
  `v0.11.4 · sdcd 0.11.4` cell, Settings → About, `sdcd --version`, `host.status` and the installer names
  (`SDC_0.11.4_x64-setup.exe`) agree - the thing `node _verify/version-report.mjs` exists to check.
* **Nothing else changed.** Everything 0.11.3 shipped is in this build unchanged: the `Reconnect` that
  measures before it claims, the Provider Hub's single `✕`, the new chat that keeps its folder, and the
  Gemini refusal that names a route which works. The release pipeline keeps one release at a time
  ("Keep only this release"), so a fresh download gets this build.

## [0.11.3] — `Reconnect` now measures, and the Hub shows one `✕`

Four reports, and three of them were about something that only *looked* like it did the job: the degraded
banner's `Reconnect` button was a `Toast` and nothing else, the Provider Hub carried two close buttons a
few pixels apart, a new chat arrived with no folder, and a Gemini turn failed with Google's own sentence
and no way out of it.

### Fixed — the banner's `Reconnect` said `Reconnected` without measuring anything

* **`host.probe` (new method): measure this host again, and put the answer on its row.** The button was
  `toast('Reconnected')` - one word, said before anything had been dialed. A machine that had come back
  (a VPS rebooted, a route repaired, a laptop reopened) stayed `offline` in the sidebar until the whole
  app was relaunched, and a machine that was *still* down was told `Reconnected` just the same, which is
  the worse half of the bug. The daemon now dials it: `connecting` at once so the dot moves, `ssh` on a
  background task, then the verdict as a `HostStatus` event on the host's own row.
* **The verdict is an event, not the call's result** - the shape `host.add` already uses, for the same
  two reasons: a synchronous probe would hold the dispatcher for as long as `ssh` takes to time out, and
  a fact about a host belongs on the host rather than in a `Toast` that the next launch replays.
* **Three answers, decided in this order.** `local` is `connected` without dialing anything; a row SDC
  has no address for is refused (`bad_request` - there is nothing to measure); a real address gets the
  measurement. It is deliberately the *probe* and not `finish_connection`: a reconnect has no password to
  spend and no key to install, because the key is already in that host's `authorized_keys`.
* **The machine line survives a look.** `upsert_host` writes `platform` unconditionally, which is right
  for the add path (that row is new) and wrong here - so a re-probe carries the host's existing
  `Debian 12 · x64` through, and looking at a host again cannot erase what it says about itself.
* **The copy moved with the behaviour.** `Reconnecting to prod-1…` while the daemon measures, and
  `Could not reconnect to prod-1` only when the call itself failed; a refusal from the daemon is reported
  in the daemon's own words, which name what is missing. `strings.main.degraded.reconnected` is gone.

### Fixed — the Provider Hub showed two `✕`

`Modal` renders the frame's close button, and the hub's header carried one of its own: the dialog showed
two crosses a few pixels apart, with two different labels (`strings.hub.close` and `strings.modal.close`)
and only one of them inside the focus trap. The header's is gone, `strings.hub.close` with it; the
frame's stays, which is the same button Settings, Add host and Permission show.

### Fixed — a Gemini that Google has cut off, and the route that still works

Measured on 2026-09-26 against a **finished** sign-in (`~/.gemini/oauth_creds.json` is there, `gemini
--version` answers 0.60.0, its settings name `oauth-personal`), so this is not the signed-out case: every
turn ends

```text
Error authenticating: IneligibleTierError: This client is no longer supported for Gemini Code Assist
for individuals. To continue using Gemini, please migrate to the Antigravity suite of products
                                                        (exit 1, empty stdout, that line on stderr)
```

Google has cut this client off for individual accounts, so the *subscription* route cannot work at all -
and the transcript showed that sentence with nothing to do about it. `gemini::refusal_sentence` recognises
the refusal (and only that one, and only for `gemini`) and answers with the route that does work: the same
models reached with a key instead, the `google` provider on `native_api`
(`generativelanguage.googleapis.com`), which SDC has had since 0.9.0. Every other failure keeps the CLI's
own words, unchanged.

### Fixed — a new chat arrived with no folder

`+ New chat` created a session with no project, so the chat *next to* the one you were working in started
folderless - and the engines would have run wherever the daemon was started, which is exactly the bug
0.7.6 fixed for the first chat and left in place for every chat after it. A new chat now inherits the
project you are working in **on that host** (`intents.ts` → `projectOn`): the active chat's folder first,
then the newest chat on that host that has one. The host half is deliberate - `session.open { projectId }`
resolves the folder on the daemon and does not check that the project belongs to that host, so a local
path can never travel to a VPS.

### Tests

* `a_probe_measures_a_host_again_and_says_so_on_its_row` (new, `sdcd/tests/lifecycle.rs`) starts the real
  daemon and asserts all three answers - including the verdict that arrives *after* the answer, which is
  what `request_until` exists for: a helper that waits for a notification instead of for the first quiet
  moment. The address is `127.0.0.1:1`, so the verdict is `offline` on any runner, with no network and no
  `ssh` required to make the point.
* `reconnectHost` has three tests of its own in `app/src/store/intents.test.ts`: which call the click
  makes, which sentence it says while the daemon measures, and both failure sentences.
* `googles_refusal_names_the_route_that_still_works` holds that captured sentence and asserts the two
  properties that matter: the way out is named, and Google's own words are kept rather than rewritten.
* Two more `newChatOnHost` tests: the folder comes from the chat you are in, and from the newest chat on
  that host when the caret's chat has none - never from another host's project.

## [0.11.2] — Gemini, researched to the bottom: the sign-in completes and the answer reads right

The report was "Gemini still does not work". This release is what running the app's own sign-in
flow against the installed Gemini CLI (0.60.0) and reading its source found - five defects, each
measured before it was fixed, and the whole sign-in verified end to end through a real `sdcd`.

### Fixed — the sign-in that could never finish

* **Nobody answered Gemini's question.** Over pipes Gemini is headless, and headless it asks
  `Opening authentication page in your browser. Do you want to continue? [Y/n]:` and waits on stdin.
  The Gemini recipe typed nothing, so every sign-in from the app sat on that question until it was
  cancelled - which is why `~/.gemini/oauth_creds.json` was never written. Recipes now carry
  `answers` - `(question, reply)` pairs typed **when the question appears** - and Gemini's answers
  `Y`. Answered, Gemini starts its loopback callback server and opens the Google page in the browser
  itself. Verified through the daemon: the question is answered in 1.5 s, a `node` listener opens on
  127.0.0.1, and the dialog reads `waiting_for_browser`.
* **The question was invisible.** The process ring read output with `read_line`, and a question has
  no newline - so neither the dialog nor anything else could ever see it. Each stream now keeps its
  half-written tail, and output includes it (`Session::partials`).
* **A new state for a CLI that opens the browser itself.** `waiting_for_browser`: no link to copy, no
  code box to paste into - the card says *"A Google sign-in page opened in your browser. Approve it
  there"* and turns to Signed in by itself when the credential appears. The finished login's
  leftover `node` (headless Gemini waits for a prompt that never comes) is closed.

### Fixed — processes that outlived their Stop

* **Windows killed the wrapper, not the CLI.** An npm CLI runs as `cmd /c gemini.cmd`, the work is
  a `node` grandchild, and `Child::kill` ended only the `cmd`: twelve Gemini processes were found
  still running, each holding an OAuth callback port. A cancelled sign-in and a stopped CLI turn now
  end the whole tree (`pty::kill_tree`, `taskkill /T`), and `CliAdapter::kill` - which only forgot
  the pid - now kills it. Verified: after cancel, zero Gemini processes remain.

### Fixed — a Gemini answer that read wrong once it arrived

Read from Gemini's `JsonStreamEventType` source, and held by a test written in its exact shapes:

* **The prompt opened every answer.** Gemini echoes the prompt as `{"type":"message","role":"user"}`
  first; the parser printed it, so each answer began with the person's own question and the whole
  replayed history. Only `role: "assistant"` messages are the answer now.
* **Every tool was one card that never finished.** Gemini names tools `tool_name`/`tool_id` with
  `parameters`; the parser read `name`/`id`, so every call became a card called "Tool" with id
  `call`. Tool cards now carry Gemini's own id, name, kind (edit / run / read) and target, and a
  `tool_result` finishes its card - failed ones red, with Gemini's error.
* **A failed turn ended green.** Gemini's `result` says `status: "error"`; it is a failure now.

## [0.11.1] — the freshest chat gets the located folder too

Same release as 0.11.0 plus one ordering fix caught right after tagging: a chat created *by* the
routing itself is not folded into the window's state yet when the locate step asks about it, so the
step treated "not found yet" as "has a folder" and skipped the bind - on exactly the chat that needed
it most. A session the window cannot see yet is a session with no folder, and it now locates.

## [0.11.0] — the domain finds its files, the sidebar finds anything, and Gemini finally answers

The release behind *"ami domain a project file access cai"*: naming a site is now enough to be
working **in that site's own folder**, two sites on one VPS are two chats working at once, and the
sidebar's search answers "where is index.php" as fast as it is typed. Plus the real end of the
Gemini story: a connected Google API key now reaches the CLI, and the browser sign-in is detected
when it finishes.

### Added — type the domain, get the site's files

* **`project.locate`** (new method): where a named thing lives on a machine. On a host it reads the
  web server's own answer first - an nginx `root` or Apache `DocumentRoot` whose server name matches
  the domain - then the conventional homes (`/var/www/<domain>`, `/srv/<domain>`, `~/<domain>`, a
  vhost layout, `/home/*/public_html`); locally it checks the usual code folders. Every candidate is
  a real directory on that machine, best-first, never a guess.
* **The prompt binds the chat to the site.** *"skilleddesk.com er file e kaj korte chai"* routes to
  the host (0.10.0) and now also **finds and binds the site's folder** when the chat has none: the
  agent starts inside `/var/www/skilleddesk.com`, not inside a blank workspace, and the sidebar's
  tree is the site the person meant.
* **Two domains, one VPS, two chats at once.** A prompt that names a saved *project* now routes to
  the chat bound to **that project** (`projectMentionedIn`) before the host matcher gets a say - so
  skilleddesk.com and example-shop.com on the same machine each get their own chat, bound to their own
  folder, running turns at the same time. A fresh chat opened this way is titled after the project,
  so the tab strip reads like the work. Works the same across several VPSes and locally.

### Added — the sidebar's search bar

* **Always there, answers as you type.** The Files section's search is no longer an icon toggle: a
  search bar sits above the tree, debounced at 300ms, and one `fs.search` now answers with **names
  and contents both** - files and folders whose name matches come first (click a file to open it, a
  folder to unfold the tree down to it - `revealFolder` loads and expands every level between), then
  the lines that contain the words. Stale answers are dropped by sequence, the box clears when the
  folder changes, and a daemon one release behind (no `files` in the answer) degrades to contents
  only instead of crashing.
* `fs.search` gained the `files` half on both machines: a bounded name walk locally, two pruned
  bounded `find`s over `ssh` on a host - `.git`, `node_modules` and `target` are skipped whole.

### Fixed — Gemini, the last mile

* **A connected Google API key now reaches the CLI.** `provider.save` stored the key and the Gemini
  CLI never saw it; a local Gemini turn now hands it over as `GEMINI_API_KEY`, so Gemini answers with
  **no browser sign-in at all**. The measured trap (0.60.0): `selectedType: oauth-personal` - which
  SDC's own sign-in prepare step writes - makes the CLI *ignore* the key and hang on its auth
  question; when that choice never produced a credential and a key exists, the turn rewrites it to
  `gemini-api-key` (`gemini::auth_plan` holds the whole measured matrix). Someone's own Vertex or
  GCA setup is never touched, and the key never travels to a remote turn.
* **The browser sign-in is detected when it finishes.** Gemini never prints a "logged in" sentence -
  it opens the browser itself and writes `~/.gemini/oauth_creds.json` when the person approves. The
  sign-in dialog watched for a sentence, so it sat at `waiting_for_url` over a login that had
  finished; it now watches for the credential file, the same fact `provider.list` reads.

## [0.10.1] — the Intel Mac build is back

Same app as 0.10.0. The 0.10.0 release published Windows, Linux and Apple Silicon installers but no
Intel Mac one: that build stopped on a lifecycle test (`a_daemon_started_by_hand_stays_where_it_is`)
whose 10-second read timeout was shorter than `host.status` takes on the Intel macOS runner. The
timeout is now 60 seconds, so every platform's installer is built again.

## [0.10.0] — type and it works: no mandatory folder, the domain routes the chat, and Gemini stops hanging

The release the screenshot forced: `native_api failed · Agent mode works inside a folder, and this
chat has none` on a fresh chat, a Gemini turn that never answered at all, and the ask behind both -
*"just promt or chat korlai ai sokol kisu kore dai"*. An agent turn now works from the very first
prompt, on this machine or on a saved VPS, with nothing to set up first.

### Fixed — the two failures in the report

* **Agent mode no longer demands a folder.** A folderless chat's agent turn used to end in a refusal
  before it started. The daemon now **provisions a workspace** instead: `~/SDC Workspaces/<chat>-<id>`
  on whichever machine the chat lives on (over `ssh` for a VPS chat), created, added as a project and
  bound to the chat in one step - the folder chip moves the moment the turn starts
  (`methods::provision_workspace`). Opening a folder by hand still works and still wins; the refusal
  survives only as the fallback for a provisioning that itself failed, and its sentence still says
  what to do.
* **A signed-out Gemini is a sentence, not a hang.** Signed out, `gemini -p … --output-format
  stream-json` prints `Opening authentication page in your browser. Do you want to continue? [Y/n]:`
  **without a newline** and waits forever - measured on Gemini CLI 0.60.0, even with stdin closed. The
  adapter read lines, so it never saw the question, never got EOF, and the turn hung with nothing on
  screen. The stream is now read **in bytes** (`engines::cli::read_structured`): a known sign-in
  question is recognised the moment it arrives, any other half-written non-JSON line gets fifteen
  seconds of silence first, and then the child is killed and the turn ends with the CLI's own question
  and the fix - *"Sign in first (Settings → Providers → gemini → Sign in)"*. The translator gives it
  its own card (`needs-person`) instead of "no rule yet".

### Added — the domain routes the chat

* **Name a saved host, work on it.** *"ami skilleddesk.com er file e kaj korte chai"* now runs the turn
  **on that host**: a prompt that names a saved VPS - by the host part of its address or by its label,
  on word boundaries, four characters or more, never the local host - is routed to that host's newest
  chat with a folder (else its newest chat, else a fresh one) before the engine starts
  (`hostMentionedIn`, `sendPrompt`). The window switches to the chat it routed to and says so in a
  toast. Combined with the workspace provisioning above, "type the domain, get the machine" is one
  step.
* **The stream says it is alive before the first token.** Between Send and the first event a turn drew
  nothing at all, and a slow first token read as a dead turn. Three pulsing dots and *"Waiting for
  <model>'s first word…"* now stand in until anything arrives (`Turn.waiting`), claiming no progress -
  only that the turn is waiting on the engine.

### Fixed — two chips that lied in split view or on a VPS

* **`Change folder` on a VPS chat opens the remote browser.** It used to open the *native* picker -
  this machine's filesystem - so every choice was refused with "not a folder on <host>". The chip now
  opens the same `fs.list` browser the Files panel uses, and the chosen folder re-points **this chat**
  (`rebindFolder`, `remoteFolderSessionId`) rather than landing in another one.
* **The folder chip reads its own pane.** In split view the chip read the active tab, so the second
  pane's chip named the first pane's folder - and re-pointed the wrong chat. The pane now passes its
  own session down (`FolderChip sessionId`).

## [0.9.0] — signs in to a hardened VPS, every big model maker, and a project from one command

The release the report "kono vabai vps a connect hoi nah" forced, plus the widening it asked for:
every original model provider connectable, model lists that update themselves, and a project started
from nothing with one dialog. Verified against the report's own VPS (keyboard-interactive with a
verification code on every session) and in the desktop window against an isolated daemon.

### Fixed — the VPS that could not be reached at all

* **Sign in once, work all day.** A host with public-key login switched off - a password and a one-time
  verification code on every connection - could never be reached by the key-install design: there was no
  key it would accept, and every `fs.list` would have needed a fresh code. `sdcd` now signs in **once**
  through a kept-open connection (`ssh::session`, ControlMaster with a 10-hour persist) and routes every
  later call through it with `-O proxy`; nothing asks again until it closes. The password and the code
  are typed into `ssh`'s own prompt through a loopback askpass helper (the daemon's own binary), used
  once, stored nowhere.
* **The window can ask for the code.** `host.add` / `host.trust` take an optional `code`; the Add-host
  form has the field, and a host that turns out to need one gets the **Sign in** card (password + code)
  instead of the key-install card - the doctor's `ssh` row says which. `host.remove` closes the host's
  signed-in connection.
* **A changed host key still refuses the sign-in.** The one-time connection pins against SDC's own
  `known_hosts` exactly like every other call; a machine presenting a different key never sees the
  password.
* **Remote turns lost their stdin.** `exec setsid` on the far side forks when the login shell is already
  a group leader, so the prompt on stdin, the answer and the exit code were all lost (`claude` said
  "Input must be provided either through stdin"). Run as a child (`setsid sh -c …`) it holds the pipe
  and the shell waits for it - measured on the report's VPS, and the reason a remote agent turn now
  edits files there.

### Fixed — a CLI agent that said Done and had written nothing

* In `--print` mode none of the three CLIs can ask a permission question, so a Write was silently
  refused: the turn ended `Done`, the transcript said "created", and the folder was empty (measured
  locally and on the host). The session's autonomy now travels to each CLI's own flags - Claude Code
  `--permission-mode acceptEdits` (Simple), `+ --allowedTools Bash` (Pro), its own full-autonomy switch
  (Auto); Codex `--sandbox workspace-write` / `--full-auto` / its bypass flag; Gemini
  `--approval-mode auto_edit` / `--yolo`. The folder is checkpointed before every change either way.

### Added — every original model provider, and a list that updates itself

* **Six new API providers**: Google Gemini API, xAI Grok, Moonshot Kimi, Mistral, Qwen (DashScope) and
  Z.ai GLM - cards in the Hub, curated models in the bundle, live `…/models` refresh, key test, and chat
  routing derived from the same block (`native_api::endpoint_for`), so a key is all a new provider needs.
* **The catalogue keeps itself fresh.** The daemon refreshes every provider that can be asked (a stored
  key, Ollama running, OpenRouter's public list) at start and every twelve hours, and the moment a key is
  saved - and pushes the new `ModelsUpdated` event, on which every open window re-reads the list. A model
  a provider ships tomorrow is in the dropdown tomorrow, with no build and no Refresh button.

### Added — start from scratch with one command

* **`project.scaffold`** makes `<parent>/<name>` on this machine or on a host and adds it as a project in
  one call (a plain name only - separators and `..` are refused). The **Start from scratch** dialog
  (empty state, palette) takes where, a name and - optionally - what to build, then opens the chat and
  starts the **agent** turn on it: folder, project, chat and first build from one button.

### Changed — the stream you watch

* **A live measured line under a running turn**: elapsed, thinking time, ~tokens and ~tokens/s (marked as
  the estimates they are), and the tool-call count - and a running tool card's pill now ticks its own
  seconds. The finished footer still carries the daemon's real totals.
* **Deltas fold once per frame.** `TurnDelta`/`ThinkingDelta`/`ToolCallOutput` are batched into one
  store update per animation frame instead of a render per token - the stream draws at the display's
  pace however fast the model talks. Order is kept: any other event flushes the buffer first.
* **The model menu searches.** A search line filters by model name, id or provider across every connected
  group - and a hit that was folded behind "Older versions" is shown, not hinted at.

### Internal

* `Prompt`/`RunPlan` carry the turn's autonomy; `providers::CATALOG` grew to 15 cards with a test that
  every API card has a bundle block; Google's `models/` id prefix is normalised; the app store's fold
  subscriber is idempotent across dev hot-reloads (two subscribers interleaved every delta twice - dev
  only, but now guarded).

## [0.8.0] — an agent that finishes the job, a second AI that checks it, and a Time Machine that restores

The v4 direction (`sdc/docs/ROADMAP-v4.md`): describe the task, and the window gets it built, run and
checked - with every change one Rewind away. Verified in the desktop window against an isolated daemon,
a real DeepSeek agent turn (file edited, `npm test` run, 5 of 5 passing, ≈$0.0047) and a real Claude Code
review of its diff.

### Added — SDC Agent: the daemon's own agent loop

* **An API or local model now does the work, not only the talking.** With **Agent** chosen in the
  composer, a native-API or Ollama model runs inside `sdcd/src/agent`: it reads, edits and runs commands in
  the chat's folder until the task is done - on this machine **or on the host** the chat is on, because the
  tools sit on the daemon's existing `fs.*` / `shell.run` / `git.*` paths. The three CLIs are agents
  already, so the switch changes nothing for them.
* **Eight tools** (`read_file`, `list_dir`, `search`, `git_diff`, `write_file`, `edit_file`,
  `run_command`, `update_plan`), both tool dialects (Anthropic content blocks with thinking and its
  signature kept; OpenAI-compatible `tool_calls` by index, which covers DeepSeek, Groq, OpenRouter and
  Ollama's `/v1`), paths confined to the folder, the deny list and the file guard underneath.
* **A permission gate that waits.** Simple asks before every change, Pro before commands, Auto only before
  a dangerous-looking one (`rm -rf`, `git push`, `curl … | sh`, `sudo` …). The dialog opens by itself and
  closing it is Deny. **Show me first** asks the agent to show the change and wait.
* **Bounds.** 25 model calls per turn (then it says how to continue), a footer with steps, tokens and the
  cost the catalogue implies (`≈$0.0047`), and a Stop that drops the connection mid-answer.
* **The plan card** (`update_plan` → `PlanUpdated`), and the **checkpoint rail**: the checkpoint a turn wrote,
  where it wrote it, with a two-click Rewind here.

### Added — Verify: the folder's own checks, then a review by another AI

* `verify.run` reads the manifests and runs what the project says "working" means - its own
  `typecheck` / `lint` / `test` / `build` scripts with its own package manager, `cargo check` / `cargo test`,
  `go vet` / `go test`, `pytest` - with `CI=true`, here or on the host.
* Then **a different engine than the author** reviews the diff since the turn's checkpoint (new files
  included) and answers with a verdict and issues pinned to `file:line`; an issue opens its file on its line,
  and **Fix with a prompt** puts the fix request in the prompt box. A failing check skips the review unless
  asked. The turn footer gets **Verify with…** and the outcome; Ctrl+Enter runs it.

### Added — the workbench

* **A real editor** (CodeMirror 6, lazy-loaded, themed from the tokens): highlighting for TS/JS, Python,
  Rust, HTML, CSS, JSON and Markdown, search, undo, line wrapping, Ctrl+S; **tabs** with an unsaved dot and a
  two-click discard.
* **The tree changes things**: New file, New folder, Rename in place, a two-click Delete, and **Search in
  folder** - each through the daemon (`fs.rename` / `fs.delete` / `fs.mkdir`, new), confined to the chat's
  folder, with a checkpoint first. The tree, the git badge and open tabs refresh after a turn or a rewind.
* **A live Preview**: type a dev server's address and the frame loads it; Reload and Open in browser work.
* **Answers render Markdown** as React elements (never HTML): code blocks with a Copy button, lists, bold,
  links that open in the browser.
* **The model menu shows only what is connected**, with how (`✓ CLI · signed in`, `✓ API key · verified`),
  the newest two versions of each family (the rest behind **Older versions**), non-chat models left out, and
  one line for the rest (`6 providers not connected · Manage in Provider Hub`). Alt+E skips engines with
  nothing connected. Current Anthropic models in the bundle.
* **Live thinking**: open and streaming while the model thinks, with a clock measured from the log, folded to
  `Thought for 6.2s` after.
* **Analytics from the log**: turns, reported cost and tokens, a seven-day chart, the share by engine.
* **A prompt queue that sends**: Send during a running turn queues (three per chat) and sends when it ends.

### Fixed — things that looked like they worked

* **Rewind never restored a file.** It ran `checkout` with the shadow repository as its own work tree,
  discarded the error and answered `restoredFiles: 1`. It now makes the folder exactly what the checkpoint
  recorded (changed files back, deleted ones returned, later files removed), goes **to** the checkpoint that
  was chosen (it restored the newest, and the newest could never be chosen), and **Redo** restores the files
  too (it only moved rows, and put them back titled `'restored'` with no hash). The window had never heard of
  a rewind: `RewindApplied` was pushed without its `type`.
* **The agent's checkpoint was taken after its change** - the turn loop checkpointed on receiving the
  tool event, but the agent had already written the file. The agent now checkpoints synchronously first.
* **Stop did not stop**: `engine.cancel` marked the turn interrupted and never told the engine. It now kills
  a CLI's process group and drops an HTTP stream between reads, and nothing the engine still says reopens
  the turn.
* **No Anthropic API turn could ever stream**: the request lacked `anthropic-version`. History also went out
  with every message as `user`, and Ollama got none.
* **After a reload every chat looked empty** (the bridge replays only past its own last `seq`): the page now
  asks for the whole log itself, and de-duplicates against the daemon's sequence rather than the log's.
* **A turn's history lost its tool calls**, so a model decided its own verified result was invented and
  apologised; the stored answer now records them.
* **Claude Code's tool cards spun for ever**: the CLI reports a finished tool as a `user` message of
  `tool_result` blocks, which the parser ignored. It reads them now (`done · 10 ln`, or `failed`), and a
  card an engine never finished stops when its turn ends.
* **Demo data and toasts where features should be**: the Verify tab's fixed rows and "3 pass, 1 fail",
  Analytics' "$4.12" and "Claude Max ~60%", the queued "also add a test for this", the permission card for
  `src/database.js`, Preview's Back/Forward/Attach, the Time Machine's "Compare two points", the toast's
  "Undo this" that only closed the toast, Esc that only toasted, and Ctrl+Enter's fake verdict.
* **Smaller**: a VPS save now takes the host's checkpoint (the toast already claimed it); the right panel's
  seven tabs no longer hide three behind a hidden scrollbar; `text-state-warning` existed nowhere (five
  components); `turn 14 Â· now` mojibake; the Time Machine listed every chat's checkpoints; Esc and Ctrl+Z
  could act on another chat; Windows paths spelled two ways broke rename.

### Not done here, and why

* **The VPS end to end on the owner's server** needs its password once (Install key); every host path above
  is the same code as local and is covered by tests, but it was not driven against that machine from here.
* **A terminal emulator** (`xterm.js` + a PTY) for full-screen programs, **provider OAuth**, **signing**, and
  a **screen-reader pass** remain on the list in `sdc/README.md`.

### Added — `Install SDC's key`: the step that finished a host whose pin was already in place

The report *"VPS connect hosse nah kono vabai"* was, on the machine it came from, **not** a broken
connection. Run against that server (`ssh -p 8443 deploy@203.0.113.10`), the daemon's own calls
say so:

```
port 22 → closed        port 8443 → open (SSH-2.0-OpenSSH_10.2p1 Ubuntu)
host.add            → untrusted · SHA256:Xk3v9Qm2b7EXAMPLEfingerprintNotARealKey0
host.trust          → pinned: true
probe               → offline · "…does not accept SDC's key yet…"
host.key (again)    → matches: true · pinned: true
pty.open            → Permission denied (keyboard-interactive)   ← the far side's own words
```

The pin was in place, the transport was fine, and **nothing had copied SDC's key onto the host** — which is
the one step that needs the password, once. That was reachable only from the *add* form, so a host whose pin
already existed had no way to finish: it stayed red for ever.

* **`host.doctor`'s `ssh` row now carries the fix** (`Install key`), and it is derived from the trust state
  rather than guessed from a sentence: unknown key → `Trust`, changed key → `Re-pin`, **pin in place and the
  probe still failed → `Install key`**, and a machine that is simply down gets no button at all;
* **the host's card asks for the password once** and installs the key with it (`AddHost` →
  `installHostKey` → `host.add` reusing the row, which is the already-tested install path), and re-runs the
  doctor so the card and the row move to the truth;
* **`user@host:8443` parses**, because the card sends the address it *shows* back to the daemon — and it is
  what a hosting panel prints. `-p` still wins when both are written, a bracketed IPv6 keeps its colons, and
  a mistyped port is refused (`host:eight` is not a hostname);
* **`ssh.key`** (67th method): the **public** half of SDC's key. It never makes one — the surface that needs
  it is the case this daemon deliberately does not automate, a host that requires a **verification code**,
  and the line a person pastes into `authorized_keys` is now shown instead of described.

### Fixed — three sentences that sent people to the wrong place

* **`ssh-keyscan`'s header was becoming the error.** On that host `ssh-keyscan` fails
  (`choose_kex: unsupported KEX method sntrup761x25519-sha512@openssh.com`) while a real `ssh` to the same
  port completes the key exchange, so the fallback handshake is what makes the host work — and when *both*
  fail the reason reported is now the handshake's (`Connection refused`, `Permission denied`), with
  ssh-keyscan's appended only when it differs. The first line of its stderr is its own `# host:port`
  comment, and that comment used to be the reason a person read;
* **`did not answer: Connection timed out` now says which door SDC knocked on.** A target written without a
  port dials 22; on that host 22 is closed and the sshd is on 8443, so the sentence read as "the machine is
  down". The hint names the port and the two ways to give another one, and it is absent when the target
  already named one;
* **the scan's fingerprint sentence** now names the **host** rather than `user@host`, matching what
  `ssh-keyscan`/`known_hosts` mean by an address.

### Added — `sdc/docs/SSH-CONNECT.md`, and `_verify/ssh-doctor.mjs`

The operational half of `REMOTE.md`: every step's real code (target parsing, the key, the scan, the pin, the
hardened argument set, the install), the exact `ssh` command to run by hand for each layer, the failure
table, and the fifteen-second diagnosis. `_verify/ssh-doctor.mjs` runs those five layers in one command and
exits non-zero when the probe fails:

```powershell
node _verify/ssh-doctor.mjs 'deploy@203.0.113.10:8443'
```

```
1. client     : OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2
2. port       : 8443 (no guess: this is the port in the target)
3. host key   : ssh-keyscan said: choose_kex: unsupported KEX method sntrup761x25519-sha512@openssh.com
4. SDC's key  : 256 SHA256:Qa+B67XMaUq4yOcvcymel5J5JIqKSF+ukXrHzCJT8MA sdc (ED25519)
5. probe      : deploy@203.0.113.10: Permission denied (keyboard-interactive)
verdict: the transport is fine; the far side refused the session → `Install SDC's key`
```


### Added — the Terminal, and a Stop that stops the whole tree

Two of the four things the previous draft of this release listed as absent were not missing *layers* but a
missing **surface** and a too-weak signal, and both are now closed.

**The panel's seventh tab, `Terminal`** (`app/src/panels/right/TerminalTab.tsx`, drawn in
`design/ui-prototype.html` with the panel's own primitives). A line typed there runs through
`shell.run { line }` — the same call an engine's `run` step uses — so it is a step in the chat: the daemon
checkpoints first, announces the tool call in the turn stream, applies the deny list, and on a chat whose
folder is on a host it runs **there**, in that folder. `Run in background` uses `pty.open { line, hostId }`
and its output keeps arriving while you look at another tab. Above the input, always, is *where the command
will run* — `~/app/landing on prod-1` — because `rm -rf build` reads the same on a laptop and on production,
and nothing else in the window makes that difference visible. A refused line lands in the entry's stderr
slot with the daemon's own sentence, which is where a terminal puts a reason.

**A remote cancel now kills the process *group*.** The turn line wraps the CLI in `setsid` (decided by the
host itself, in the same round trip: `if command -v setsid …`), so the pid in the pid file is a group
leader's; `engine.cancel` sends `kill -TERM -<pid>`, waits a second, then `kill -KILL -<pid>`. Before, a
`kill -TERM <pid>` reached the CLI alone: a test runner or dev server it had started kept going on
somebody's server after the turn said *interrupted*. The `~/.sdc/run` directory is also created by the line
now — without it the pid file was never written on a fresh host, which quietly left `Stop` with nothing to
signal. `pty.close` uses the same group kill for a background process, and both fire on a detached thread so
a Stop button never waits on a slow link.

**`Install` on a host's doctor row is now a path, not a dead end.** It opens the Terminal **on that host**
(focusing one of its chats first, so the command runs in *that* folder on *that* machine) and the person
runs the installer. SDC deliberately does not `npm i -g` anything over `ssh`: that writes to somebody's
server as their user. What the button offers instead is every guard rail SDC has — the deny list, the
checkpoint taken before the command, and the tool-call pair in the session's log.

**A whole line is guarded statement by statement** (`pty::denied_reason_line`). `shell.run` and `pty.open`
now accept a `line` as well as a program plus arguments — that is what a terminal has — and a guard anchored
at the first word would have waved `git status && shutdown /s` through. Each statement (`;`, `&&`, `||`,
`|`, `&`, newline) is checked where a program would be, with the same one level of shell unwrapping the
program form has, and the shell that runs a line is the **host's** on a host (`sh`) rather than the one the
window happens to be running on.


### Added — the engines, the checkpoints and the rewind run **on the host** too

The three things §5 of the first draft of this release called absent are in, and the reason they could be
is that none of them needed a second daemon:

* **`engine.start` on a chat whose folder is a host** spawns an `ssh` whose remote command is
  `cd <folder> && sh -c 'echo $$ > <pid file>; exec env … <cli> …'` (`engines/cli.rs::remote_command`).
  The prompt still travels on stdin, the CLI's own JSON stream still arrives on stdout, so the parsing,
  the events and the window are unchanged - and the pid file means `engine.cancel` is a real
  `kill -TERM` over a second `ssh` (on a detached thread, so a Stop button never waits on a slow link)
  rather than a closed pipe that leaves `claude` running on somebody's VPS;
* **checkpoints and rewind** commit to a shadow git repository **on the folder's own machine**
  (`$HOME/.sdc/git/<hash of the root>`, the same derivation the local one uses), so a save, a
  `shell.run` and a turn's checkpoint all hash the host's files, and `rewind.apply` restores them with
  `git checkout <sha> -- .` there. The new `checkpoints::Snapshot` enum (`Unbound` / `Local` / `Remote`)
  makes the compiler ask every call site which machine it means, which is what stops a remote project
  from being hashed against a local path;
* **`host.doctor { hostId }` about a host** now answers about *it*: reachable, key pinned, `git` and the
  three CLIs present **there**, `$HOME` writable, and the chat's folder when one is named. Before this it
  returned the ten local checks for every host - ten rows about the laptop under a heading that said the
  VPS's name.

### Added — `host.key`, and the Re-pin the prototype draws

`host.add` asks the trust question once, in the same breath as adding the host - so a window that was not
open at that moment (a relaunch, a second window, a host added days ago) had the row, the sentence, and
no fingerprint: a button needs a value, and a paragraph is not one. `host.key` is that value: it scans
(no authentication), answers `{hostKey, keyType, pinned, matches, pinnedKey}`, and pushes a `HostStatus`
when the answer changes what the host's row says. A key that **changed** answers `matches: false` with
the fingerprint the machine presents *now*, which is what `Re-pin` confirms - the doctor row the
prototype has drawn since before there was a doctor (`SSH to prod-1 · host key changed — needs re-pin`,
fix `Re-pin`).

The window got one surface for all of it: the Add-host dialog can be opened **about a host** (the
switcher's new key button, or a doctor row's `Trust`/`Re-pin`), where it shows the fingerprint, both
fingerprints when the key changed, the button that follows from the state, and the host's own doctor rows
under `On that host`. Fix buttons now tell the truth: `Trust`/`Re-pin` act, and `Install`/`Kill process`
say where to do it instead of toasting `Install: done` while installing nothing.

`session.list` also carries the pinned fingerprint now, so a host's card can say `Key pinned: SHA256:…`
without an event to read it from - and `host_type` maps the `hosts.kind` column (`ssh`) onto the
protocol's own word (`vps`), which is what the same host was called in a list and in an event.

## [0.7.13] — the layers a VPS needed, and the four that were missing

The report: *"vps connect korai jasse nah. VPS connection ar jonno je sokol layer proyojon sai rokom kono
kisui aikhane nai."* 0.7.0 had built one layer of eight - a target parser, a key, a one-time install and
a probe - and called it VPS support. This release builds the rest, and `sdc/docs/REMOTE.md` is the
research behind it: how VS Code Remote-SSH is actually put together (the local `ssh` binary, a server on
the far side, an `ssh -L` tunnel, and a **pinned host key** that is refused when it changes), what SDC
had, and what each layer is here.

### Fixed — the port was thrown away, so a VPS on 8443 could only ever connect once

`host.add` parsed `ssh -p 8443 user@host` correctly and then wrote only `user@host` into `hosts.target`.
The probe used the parsed target, so the *first* connection worked and everything after it would have
gone to port 22 - the second half of the report, and a bug no sentence in the UI could explain. There
are two columns now (`0003-host-ssh`: `port`, `host_key`), `set_host_address` is the one writer,
`session.list` answers with the port, and a host card says `root@vps.example:8443`.

### Added — the host key is a pin, and a changed one is an error

The old probe ran with `StrictHostKeyChecking=accept-new` against the **user's** `known_hosts`: whatever
answered first was trusted for ever, a changed key was invisible, and nobody was ever shown a
fingerprint. Now:

* `ssh::hostkey::scan` asks with `ssh-keyscan` (a key exchange and **no authentication**, so nothing is
  offered before the machine's identity is decided - and `ssh-keyscan` wants a *host*, not `user@host`,
  which a test caught: OpenSSH 9.5 answers `getaddrinfo git@github.com: A non-recoverable error`),
  falling back to a throwaway handshake into a temporary pin file where `ssh-keyscan` is missing;
* the fingerprints are `SHA256:…` **exactly as OpenSSH prints them** - a cross-check against
  `ssh-keygen -lf` runs in the test suite - so a person can compare what the dialog shows with their own
  terminal (`ssh-keyscan <host> | ssh-keygen -lf -`);
* pins live in SDC's own `<data>/ssh/known_hosts` (`0600`, in a `0700` directory), not in the user's;
* `host.add` records a host whose key is unknown as **`untrusted`** and puts the fingerprint in the
  answer and in the event; a host whose key is *not* the pinned one is refused with both fingerprints,
  and there is no "continue anyway" anywhere;
* `host.trust` is the answer: it scans **again** (a key that changes while the card is on screen is
  refused, not pinned), pins only the key whose fingerprint was shown, and only then spends the password.

And the rule that follows from it: **a password is never typed into a host whose key is not pinned**. The
install used to run against `accept-new`; it now uses `ssh::Ssh::install_args`, which differs from every
other call in three flags (a prompt is allowed) and in nothing else, so a machine presenting a different
key never sees the password at all.

### Added — a folder on a host is a folder: `fs.*`, `git.*` and `shell.run` on the far side

`fs.read`, `fs.write`, `fs.list`, `fs.stat`, `fs.search`, `git.status`, `git.diff`, `shell.run` and
`project.add` take a `hostId` (or the session's host), and on an SSH host they answer from that machine
with **the same shapes** the local implementations answer with - so the sidebar's tree, the Preview
editor, the Save button and the Diff badge are the same code on a VPS and on the laptop:

* the fork in the path is `remote_for(envelope)`; `ssh::ops` is the far side;
* the listing is a `printf '%s\t%s\t%s\n'` loop (a format this daemon defines, rather than `ls -l`'s
  columns, which differ between GNU and BSD), and a name that cannot be represented in one line is
  **counted** as hidden rather than dropped;
* `fs.read` asks the size first, so a 200 MB log is never pulled over the link to be thrown away, and a
  truncated read still hashes the **whole** file (`sha256sum`, or `shasum -a 256` where that is what the
  host has);
* `fs.write` sends the text on `ssh`'s **stdin** (`cat > <path>`), so nothing in a file is a word in a
  shell command;
* `git.status` answers for the *project's own* repository on that machine, and `("", 0)` - no badge - for
  a folder that is not one;
* the file guard runs on the remote path too: `.env`, `*.pem`, `id_rsa` and `credentials` are refused and
  counted on `/srv/app` exactly as on `H:\app`;
* every path and argument that reaches a remote shell goes through `ssh::sh_quote`, and a relative path
  is refused with a sentence instead of resolving against whatever directory a remote shell started in;
* `shell.run` keeps the deny list on the far side - a `shutdown` refused here has no business reaching
  somebody's VPS.

`project.add` with a host validates the folder with `test -d` **on that machine**, so a typo is refused
by the host that has the folder rather than by the laptop that does not. A chat whose folder is on a host
can be browsed, read, edited, diffed, shelled into - and, in the same release, **run its engines there**
(L9) and **keep its checkpoints and rewind there** (L10), because none of that needed a second daemon.

### Added — the window: the trust step, and a folder browser for a host

The Add-host dialog does not close the moment `host.add` answers any more. It watches the host's row
(folded from `HostStatus`, never held locally) and shows the daemon's own sentence: `connecting` while
the scan runs, then either the **trust card** - the fingerprint in monospace, what to compare it with,
and `Trust and connect` - or the refusal. `Cancel`/`✕`/`Escape` forget a half-finished flow.

`Open folder` cannot be used on a host: `tauri-plugin-dialog` shows *this* machine's filesystem. The
Files section's empty state grows `Open a folder on <host>`, which opens `RemoteFolder` - the host's home
first, one level per click, a path that can also be typed (`~/app` works) - and the chosen folder goes
through `project.add` with that host id.

### Fixed — a host's sentences were travelling in `platform`

`event::host_status` had one field for two jobs, so a machine line and a sentence were the same string:
`copying SDC's key with that password…` was pushed as the host's `platform`, and About's `This host` row
read it. There is a `detail` field now (and `hostKey` for the fingerprint), `platform` is a machine line
or nothing - the guess `linux · x64` on a VPS nobody had looked at is gone - and the five host states
(`untrusted` is the new one) map onto the waiting colour, because a question is what orange already
means everywhere else in this window.

### Added — `sdc/docs/REMOTE.md`

The research and the design in one file: how Remote-SSH works and why each piece exists, the eight layers
with the file each lives in, the seven security rules with the reason for each, the Terminal surface and
what it is not, and what is deliberately absent - each of those with the condition that would change the
answer. The `sdcd`-on-the-host tunnel is a table now rather than a sentence: what a far-side daemon would
buy (a PTY, file watching, a cache) is either **already done over exec** — `pty.open` on a host is an `ssh`
with a pid file and a group kill — or a feature the window does not have locally either. `sdc/README.md`'s
federated-host entry says what is true: an added host is a machine whose files, git, shell, engines,
checkpoints and terminal work.

### Verified

`sdcd`: **175 unit + 9 lifecycle + 2 streaming + 6 VCR** tests, clippy clean under `-D warnings` (one
`--ignored` test makes SDC's key on purpose). The new tests pin the things that quietly break: the
hardened flag set (including that `accept-new` never returns), `sh_quote` on hostile names, the
fingerprint against `ssh-keygen -lf`, the known-hosts parser and its `[host]:port` lookup, the listing
and hit parsers, the guard running **before** a connection is opened, the trust/changed-key sentences, the
remote turn line (the port, the chat's folder, the pid file, the CLI's arguments inside the line rather
than as `ssh` arguments, the `setsid` branch and the `mkdir` that makes the pid file possible), the
group-then-pid-then-`KILL` escalation, and the line guard. `app`: 93 vitest, typecheck/lint clean,
`protocol/check.mjs` green at **67 methods, 26 events**. A live run against the host from the original
report is in the CHANGELOG's first entry of this release: the pin, the probe's sentence and the far side's
own `Permission denied (keyboard-interactive)`.
And the whole trust flow was run against a **real sshd** (`_verify/probe-remote.mjs`, which drives
`host.add` → `HostStatus untrusted` → `host.trust` → the probe): the fingerprint the daemon printed is
character-for-character what `ssh-keygen -lf` prints for that host's key, the pin landed in SDC's own
`known_hosts`, and the sentence that came back afterwards was about **authentication** -
`does not accept SDC's key yet` - not about the host key, which is how a pin is supposed to behave.

## [0.7.12] — what CI caught that no local run could

0.7.10 shipped with a bug that only exists on unix, and this release is the fix plus the three things the hunt
for it turned up. The CI log is not readable without being the repository's owner, so the *gate* had to learn
to talk: a failing `cargo test` now puts its compiler errors, its failing test names and its panic lines into
GitHub **annotations**, which are readable. That is how the cause was found in one run instead of five.

### Fixed — the keys directory was `0600`, which clears the execute bit

`restrict_to_owner` applied `0600` to the *directory* as well as to the key file. On unix that removes the
**execute** bit, and without execute the owner cannot create or open a file *inside* that directory at all - so
`set()` failed with `EACCES` and four tests panicked (`auth::keychain::tests::round_trips…` and
`::an_empty_secret_deletes_the_entry`, plus `providers::tests::saving_a_key_writes_the_keychain…` and
`::removing_a_provider_forgets_the_secret…`). Windows was happy throughout, because its ACL grants full control.
The mode is now `0700` for a directory and `0600` for a file, and a unix-only test asserts both *and* the round
trip, so the next regression fails on the machine it happens on.

The previous version of this code ignored the failure (it was `let _ = set_permissions(…)`), which is why the
bug had never been seen: 0.7.10 turned the ignored result into a returned error, and CI said so.

### Fixed — a button inside a button in the sidebar's host header

`nested-interactive`, which axe calls a **serious** violation: the host row was a `div role="button"` wrapping
the `+` and the remove button. A control inside a control is not reachable by keyboard and is announced as one
thing where there are three. The header is a plain container now and the fold/unfold is its own button, named
after the host it folds.

This one is worth a note about *when* the audit found it: the first green run was green because that install had
no chats to show, and the violation needs a host with rows under it. The audit's honesty depends on the state
the app opens with - which is exactly why 0.7.10's "0 violations" was not the whole truth, and why this release
re-ran it against a populated window.

### Fixed — two copies of "where is the browser"

`smoke-bundle.mjs` and `a11y-bundle.mjs` each carried their own list of browser paths, and they disagreed:
smoke knew about `/Applications/Microsoft Edge.app`, a11y did not. On the macOS runner - which has Edge and no
Chrome - the window rendered and the audit could not start, so the release job failed. One list now, in
`app/scripts/browser.mjs`, used by both.

### Changed — the release prunes releases, not tags

The "keep only this release" step called `gh release delete --cleanup-tag`, so pruning a release also deleted
its **tag** - and a version is a point in history a fresh clone should carry, not an attachment. It deletes the
release and keeps the tag now.

### Verified

* `sdcd`: 167 tests, clippy clean with and without `--features keychain`. The four keychain tests that failed on
  Linux and macOS pass there now, as reported by CI's own annotations.
* `app`: 73 vitest cases, typecheck and lint clean, `pnpm smoke` green, and `pnpm smoke:a11y` at **0
  violations** - measured against the window that produced the `nested-interactive` finding, not a fresh one.
* CI on Linux: every step green, including `Checks (protocol)`, `Checks (cargo test)` and the axe audit inside
  "The built window renders" - which is the first time the audit has run in the release pipeline.



## [0.7.11] — four sentences in this README that were no longer true

Not a feature release. The README's "What is deliberately absent" is the part of the documentation that is
supposed to cost something to write, and four of its claims had quietly stopped matching the code:

* "**A TLS client for the native API** … the `https` transport is the next crate to add (`rustls` + `hyper`)"
  - `post_https` has streamed `https://` over `ureq` (rustls + webpki roots) since **0.6.1**;
* "a remote `https://` endpoint answers *a TLS client is not linked in this build*" - that sentence is not in
  the code at all;
* "**a remote list is `bundled` (or `cached`)** … because this build cannot reach without a TLS client" - the
  model list is fetched live, and the SQLite cache in a working install carries `fetchedAt` timestamps from
  real fetches;
* "**`provider.test` … carries `verified: false`** because this build cannot reach an `https://` endpoint" -
  it really contacts the provider and answers `verified: true`.

The fix is the measurement, not a re-reading. `_verify/live-provider-test.mjs` makes one live round trip over
TLS with a key nobody would want:

```text
$ node _verify/live-provider-test.mjs
{ "result": { "verified": true, "error": "API key is invalid. (401)" } }
```

That is the whole point of the field: the request crossed TLS, presented the key, and read the provider's own
answer about it. A `verified: false` here would have meant "we never asked".

### Changed

The four entries above are rewritten to say what the build does, and the next-step list loses the item that
was already done. What the README now admits instead is narrower and true: a **streaming** turn against a paid
`https://` endpoint has not been exercised from here (the transport under it is the same agent, and the
`http://` loopback path is what `tests/streaming.rs` drives without a network), and the provider OAuth exchange
needs a client registration with each provider - a person's job rather than a build's.

### Verified

Nothing in `sdcd` or the app changed, so this release is verified the way the others are, with the version
string as the only difference in the artifacts: 166 daemon tests, clippy clean, 73 vitest cases, typecheck and
lint clean, `protocol/check.mjs` at 0 differences, the built window renders, the axe audit reports 0
violations, the window reports `v0.7.11 · sdcd 0.7.11`, and the four probes (`files`, `078`, `folder`, `075`)
still pass against the installed build - plus the live provider call above, which is the evidence this release
is about.



## [0.7.10] — the OS key store, a protocol that cannot drift silently, and the audit that was promised

Three of the items on this README's next-step list, and one of them found more than it went looking for.

### Added — the OS keychain is used by the shipped daemon, and the fallback is protected

`keychain` compiled for months behind a feature that was off, and `README` said why. Turning it on was not a
one-line change, and the reasons are the interesting part:

* **`keyring` v3 enables no backend at all by default.** The feature alone compiles everywhere and stores
  nothing - which is worse than the file fallback, because `backend()` would answer `"os"` while every save
  failed. The backends are now chosen per platform: `windows-native` (DPAPI) and `apple-native` (Keychain).
  Linux keeps the documented file fallback, because the Secret Service needs `dbus` headers and a running
  session bus, and a daemon that cannot start on a headless box is worse than one that says which store it
  used;
* **a compiled-in store can still be unreachable** (a locked Keychain, a Windows service account). `backend()`
  is a *runtime* answer now: the store is probed once, and when it says no the file fallback is what
  `set`/`get`/`delete` use - so the reported store and the used store cannot disagree;
* **the Windows fallback had no protection at all.** "0600 on unix" was in the README next to "Windows has no
  ACL applied yet", and a key written into `%APPDATA%` inherited that folder's permissions - every account in
  `Users` on a shared machine. `restrict_to_owner` now applies `icacls /inheritance:r /grant:r
  <account>:F` to the file (and the directory, so new files inherit it). The first version passed `(OI)(CI)F`
  on a *file* as well, which marks an ACE as "for children only" and left the file with **no effective
  permission at all** - the owner could not read the key it had just written. The round-trip test caught it;
* `host.status` reports `keyProtection` next to `keychain`, because "the fallback" is not one thing: `acl`,
  `mode` or `os`.

### Added — `protocol/check.mjs`, and what parsing the schema for the first time revealed

`protocol/types.ts` is what every line of the app imports; `sdcp.schema.json` is what the daemon and the spec
are written against; nothing had ever compared them, and - it turns out - **nothing had ever parsed the
schema either**. The check does, and on its first run:

* **`sdcp.schema.json` was not valid JSON.** Three entries wrote an optional array as `["string"]?`, which no
  JSON parser accepts. Fixed (a type expression, `"string[]?"`, like the enums the file already uses);
* **nine methods were named with no `params`/`result` shape** (`session.list`, `session.fork`, `fs.stat`,
  `git.status`, `git.worktree`, `pty.write`, `pty.resize`, `pty.close`, `event.append`). All nine now describe
  what the daemon actually answers;
* and **`pty.resize` answered `{}`** - a promise to resize a pty in a build whose "pty" is a pipe runner with
  no window to resize. It answers `unsupported` with that sentence now.

The check compares the schema's names, shapes and event types against `SdcpMethod`, `SdcpMethodMap`,
`SdcpEvent` **and the daemon's dispatch table**, so a method declared in one place and missing in another is a
red build rather than a `Property 'x' does not exist` in some later change. It is now part of the CI gate.
It is a checker rather than a generator on purpose: `types.ts` carries the prose that explains each method,
and a generator would write that prose away.

### Added — an axe-core audit, in CI, and the three findings it made

The README has said since 0.6 that "the automated audit has not been run in CI yet". `app/scripts/a11y-bundle.mjs`
runs it the way `smoke-bundle.mjs` runs the window: serve the built `dist`, load `axe-core` **into the app's own
frame** (colour-contrast needs the real computed styles), wait for the shell to mount, and write the verdict into
the page for `--dump-dom` to print - no CDP, so it runs in CI. `serious` and `critical` violations fail the build;
`moderate`/`minor` are printed with their counts so "we know" is in the log.

It found three things on its first run, all fixed:

* **`aria-selected` on six plain `button`s** (critical). A tab's state on an element that is not allowed to
  carry it: the right panel's tabs are a real `role="tablist"` with `role="tab"`, `aria-controls` and
  `role="tabpanel"` + `aria-labelledby` back now;
* **white on the accent at 2.74:1 and 2.30:1** (serious) - the New chat button, the send button, the selected
  model chip, the unread badge and the `⌘N` chip. One token cannot be both a readable *text* colour on a dark
  surface and a readable *background* for white, so the fill gets its own token (`--accent-fill`, 4.96:1 with
  white; 5.52:1 in the light theme) and `--accent` keeps the value it was chosen for;
* **`--text-muted` at 3.27:1** (serious) on the surfaces it is used on. Raised to `#737F94`: 4.63-4.81:1,
  still clearly below `--text-secondary`'s 7.17:1, so the hierarchy survives.

### Verified

* `sdcd`: 166 tests, including the Windows ACL test (`icacls` output has the owner and no `Users`/`Everyone`
  entry) and the four keychain tests both with and without the feature. Clippy clean with **and** without
  `--features keychain`.
* `app`: 73 vitest cases, typecheck and lint clean, `pnpm smoke` green (the token changes did not disturb the
  painted-controls check), and `pnpm smoke:a11y` reports **0 violations** over WCAG 2.0/2.1 A + AA.
* `sdc`'s CI gate is now: typecheck, lint, vitest, `node protocol/check.mjs`, `cargo test` - then the built
  window renders, then the same window is audited.



## [0.7.9] — editing: Save (with a checkpoint first) and the diff of what changed

0.7.7 made the folder *visible*; three methods stayed unreachable from the window - `fs.write`, `git.status`
and `git.diff` - and with them the answers to "can I fix this line here?" and "what did the turn change?".

### Added — Edit and Save in the file view

A file opened from the tree gets an **Edit** button: the text becomes a `<textarea>`, **Save** writes it
(`fs.write`) and **Cancel** puts the read text back.

**The daemon takes the checkpoint, before the write.** `fs.write` now honours principle P5 itself: given a
`sessionId` it resolves the chat's folder (0.7.6), hashes the files that are about to change, pushes
`CheckpointSaved` - and only then writes a byte. The rule is enforced where the file is changed rather than by
the caller, because a rule the caller enforces is a rule the next caller forgets; `shell.run` has taken that
path since 0.7.0 and this is the same one. A caller with **no** session takes no checkpoint: there is nothing
to checkpoint against, and an orphan row in the Time Machine would be worse than none.

A file the daemon had to **truncate** cannot be edited at all: saving the visible megabyte over the whole file
would silently delete the rest of it. The button is absent and the meta line says why.

### Added — the branch, the changed count, and the patch

* the Files header carries **`main · 3 changed`** (or `main · clean`) from `git.status`, and the Diff button
  appears beside it when there is something to see. A folder that is not a repository shows neither - no
  badge, no error, because a folder without git is a normal folder;
* **Diff** opens the patch in the Preview: `+`, `-` and `@@` lines tinted by their first character, the branch
  named in the header, and nothing parsed into a diff model - a hand-rolled parser that disagrees with `git`
  about a rename or a binary file is worse than no parser;
* both take the **session's** folder, the contract 0.7.6 gave the file tools, so the window sends a chat id and
  the daemon knows where to look.

### Changed — two protocol entries that had been lying

`fs.write`'s declared result carried a `checkpointId` the daemon has never answered (the checkpoint travels as
a `CheckpointSaved` event, which is where the Time Machine reads it) and `git.diff` demanded a `sessionId` while
the daemon accepted a `root` too. Both are now what the daemon actually does, with the params it actually takes.

### Fixed — `git.status` was describing the **shadow** repository

The probe caught this on its first run, and it is the oldest lie in this release: `git.status` ran
`git rev-parse --abbrev-ref HEAD` through `run`, which passes the *shadow* repository's `--git-dir`. So a
project on `main` was told **`master`** - the branch a bare `git init` leaves behind - and a folder that is not
a repository at all got a confident `master · clean`. The badge asks "which branch is my project on?" and was
answering about a repository its owner has never heard of.

Now `git.status` asks the project's own repository (or the one above it, the way a shell would) and answers
**no branch and no count** for a folder that has none, which the window draws as no badge at all. `git.diff`
prefers the project's own repository too, and keeps the shadow as the fallback - a checkpoint's diff is the
only history a folder without git has. Two daemon tests pin both halves, and the badge is re-read on every
Refresh and after every Save, because a status read once when the folder opened is stale the moment it
matters.

### Verified

* `sdcd`: 165 tests, including `a_save_takes_a_checkpoint_before_it_changes_the_file` against the real binary -
  a Save pushes `CheckpointSaved` with a **non-empty files hash** (only possible because the chat's folder was
  resolved), `fs.read` then shows the new text, and a write with no session pushes no checkpoint at all. Two
  more pin the `git.status` fix: a repository of its own reports its own branch (`probe-branch`, not the
  shadow's `master`) and its own working tree, a folder without one reports neither, and `git.diff` comes from
  the project's repository when there is one. Clippy clean.
* **`app` 73 vitest cases (six new: Save sends the chat's id, takes the daemon's new hash, and re-reads
  `git.status` so the badge and the Diff button cannot go stale; a refused save leaves the file as it was;
  `git.status` fills the badge; a non-repository clears it; and the diff opens and closes).
* `_verify/probe-files.mjs` now also drives the whole editing path in the running window: it makes its fixture a
  **git repository** with one commit, opens a file, edits it, presses Save, checks the file shows what was
  saved, that the toast says a checkpoint was taken first, that the git badge went from `0` to `1` changed, and
  that the patch from `git.diff` carries the line that was written.

## [0.7.8] — two methods the schema declared and the daemon never answered

Both of these were *declared* in `sdcp.schema.json` and `protocol/types.ts` from the beginning, and both were
on the README's next-step list with the honest note that nothing was behind them. Neither needed a new idea;
they needed the code that should have been there.

### Added — `session.fork`

The daemon answered `unknown method`, so the fork the spec draws beside a session had no implementation and
the app had no button. Now:

* `session.fork { sessionId, atTurn? }` → `{ sessionId, turns, title }`. A fork is a new chat with the same
  folder and the same conversation **up to a turn**;
* the turns are **copied into the fork's own rows** (`Store::copy_turns`, new ids `f<parent>-<ordinal>`), so a
  rewind in the fork cannot reach back into the parent and a reload shows the fork's conversation;
* `atTurn` is **inclusive** (that is what "fork from here" means when a person points at a turn) and omitting
  it forks the whole conversation;
* the copied turns are **replayed into the log** (`TurnStarted` / `TurnDelta` / `TurnCompleted`). The window's
  transcript *is* the log, so a fork whose history existed only in the database would look like an empty chat;
  a turn that was `running` in the parent is copied and replayed as `done`, because nothing is running in the
  fork;
* the window: a **Fork** button on every session row (it appears on hover beside Rename and Delete) and a
  `Fork this chat` palette row. The fork opens in its own tab and says how many turns came with it.

### Added — `cli.recipes` in the Connect dialog

The daemon has answered `cli.recipes` since 0.7.0 - `installed` comes from the same `doctor::has` the
environment doctor uses - and **no line of the app read it**. So Connect on a subscription provider whose CLI
was missing launched the login and then reported the failure: the sentence a person needed ("install
`claude`") arrived as the explanation of something that had already gone wrong.

Now the dialog reads the recipe **before** it starts anything and shows a row: `` `claude` is installed `` or
`` `claude` is not installed `` plus the daemon's own install words, a **Copy** button and **Check again**.
The sign-in no longer starts itself for a program that is not there (that is the point of reading the recipe
first), while the `Sign in` button stays where it is for a machine the doctor cannot see into. The row is drawn
in both cases: "it is installed" is information too, and a row that only appeared on failure is a row nobody
can find when it matters.

### Verified

* `sdcd`: 162 tests, including `a_forked_chat_carries_the_conversation_that_came_before_it` against the real
  binary - two turns, a fork at turn 1 that copies exactly one, the `SessionOpened` and the replayed
  `TurnStarted` for the fork, a whole-conversation fork with no `atTurn`, and both chats in `session.list`
  afterwards. Clippy clean.
* `app`: 67 vitest cases (five new: the recipe is picked per provider and answers `null` for an API-key one and
  for an unreachable daemon; the fork's call carries the chat's id and its answer's `sessionId` is what a tab
  opens on, or `null` so no tab is opened on nothing).
* `_verify/probe-078.mjs`: in the running window - a real row is forked, the fork arrives titled
  `<title> (fork)` and open, the probe deletes it again; then the hub is opened and the recipe row is compared
  with the daemon's own `cli.recipes` answer (program and `installed`), which is the part that cannot be faked
  by a hardcoded sentence.

## [0.7.7] — the folder a chat works in, now visible: a file tree and a file viewer

0.7.6 gave a chat a working directory and put its name in the prompt toolbar, so *which* folder a turn runs
in became a question with an answer. What was still missing was everything that *looks inside it*: `fs.list`
and `fs.read` had been real methods since the schema was written and **no line of the app called either**
(README, "What is deliberately absent"). This release is that caller.

### Added — the sidebar's Files section

A tree under the session list, showing **the active chat's folder** - not a list of folders, because a chat
works in one directory (0.7.6), so the tree follows the session the way the pane does and switching chats
switches the tree. A chat with no folder says how to get one (`Open a folder to see its files`).

* **lazy, one `fs.list` per folder that is opened.** The daemon's listing is one level deep and says which
  rows are folders, so opening `node_modules` costs one request rather than a walk;
* **folders first, then files**, each sorted by name - a tree convention, decided in the window because the
  daemon's listing is name-sorted;
* **the folder's name is the row and the whole path is its tooltip**, the same rule the prompt chip follows;
* **a Refresh button** (and a `files.refresh` palette row) for a file an engine just wrote;
* **the guard's hidden names are counted out loud**: the daemon refuses `.env`, `*.pem` and its own data
  directory, and now answers how many names it kept out, so a folder with a `.env` in it says
  `1 name hidden` instead of being quietly one row short.

### Added — `fs.read` opens a file in the Preview tab

Clicking a file reads it (`fs.read`) and shows it in the right panel's Preview: the tab switches, **the panel
unfolds if it was folded** - a click that shows nothing is the kind of lie this build keeps removing - and the
file's own text appears in the app's mono type, with its name, its **whole path**, `2.4 KB · 128 lines` and
the first eight characters of its `sha256` (the same hash a checkpoint stores). No syntax highlighting: there
is no highlighter in this build, and a hand-rolled approximation would colour the wrong tokens.

### Changed — `fs.list` takes a session, and both reads say when they are cut

* `fs.list`'s `path` is **optional** now: with `sessionId` instead of a path it lists the **session's**
  folder, which is the contract `git.*` and `fs.search` have had since 0.7.6 (the window knows the chat, the
  daemon knows the folder). Its answer names the directory it listed and counts the hidden names.
* `fs.read` caps the text at **1 MiB** and answers `bytes` (the file's real size) and `truncated`, so a large
  file says `First 1 MB of 12.4 MB` rather than looking complete. The `sha256` is still computed over the
  **whole file** (`fs::hash_file`, streaming): a hash of the first megabyte would be a hash of something that
  is not the file.

### Verified

* `sdcd`: 161 tests, including two new `fs` unit tests (a listing says which row is a folder and how big it is;
  a capped read reports the real size and the whole-file hash) and the lifecycle test extended to drive the
  tree's own path: `fs.list { sessionId }` names the folder and hides `.env`, a 1.2 MB file comes back
  `truncated: true` with `bytes: 1200000` and exactly one megabyte of text. Clippy clean.
* `app`: 66 vitest cases (four new: the root read names no path and takes its root from the answer; a folder is
  read once however often it is toggled; a file click switches the panel to Preview *and* unfolds it; a refused
  read shows the daemon's own sentence and leaves the tree standing). Typecheck and lint at zero.
* `_verify/probe-files.mjs`: in the running window - a chat is pointed at a fixture folder, the tree names it,
  lists `README.md` as a file and `src` as a folder, says `1 name hidden` for the fixture's `.env`, expands
  `src` lazily to reveal `main.ts`, opens `main.ts` into the Preview with the panel unfolded, closes it with
  the ×, and deletes the chat it made.

## [0.7.6] — "chat er kono folder nai": a chat works in a folder now

*"GUI file-picker nai. aita soho aro important kisu nai jeta vs code a thake"* — 0.7.5 fixed the picker.
The other half of that report is this one: **which directory a chat works in**. A chat had no working
directory at all, so `sdcd` ran every engine in whatever folder it had been started from.

### Fixed — a chat had no working directory

The daemon's schema has had `projects` and `sessions.project_id` since the first migration and **nothing
ever wrote either row**. `engine.start` built a `Command` with no `current_dir`, so a `claude_code` turn in
a chat called `SDC` ran in the daemon's own directory - the folder someone happened to start the daemon
from, and not the project on screen. Three more things followed from the same hole:

* `checkpoint_create` and `rewind_apply` took the root as a **parameter the app never sent**, so a
  checkpoint hashed no files (`files_hash` was of nothing) and a rewind restored the conversation only -
  the file half of a rewind had never once run;
* `git.status` and `git.diff` refused with `` `root` is required `` unless the caller named a directory,
  which neither the app nor any tool caller knew;
* `session.list` had no `projectId`, so even a chat that *had* a folder would have lost it on the next
  reload.

### Fixed — deleting a chat that had been used failed

The 0.7.6 probe deleted the sidebar's first row instead of a freshly made chat, and the daemon answered in its
own words: `FOREIGN KEY constraint failed`. `session.close` was `DELETE FROM sessions WHERE id = ?` while
`foreign_keys` is ON and a turn, a checkpoint, a rewind entry and a permission row each *reference* their
session - so **every chat that had run a turn** was undeletable, and the Delete button showed a constraint
error as a toast with the chat still there. It hid for as long as it did because it was only ever tested
against a chat that had just been made (which has no turns). `delete_session` now deletes the children first,
the same four statements `delete_host` has used for a whole host since 0.6.1, and
`a_chat_that_has_run_a_turn_can_be_deleted` in `tests/lifecycle.rs` is the regression test: a turn is started
against a chat, the chat is closed, and the list is asked afterwards.

### Added — `project.add`, `project.list`, `project.remove`

Three additive methods, in the shape the rest of the protocol already uses (`session.list` is a read that
answers rows; a list is a read's result, not a stream of events):

```
project.add     { hostId?, root, name? }  ->  { projectId, hostId, root, name }
project.list    { }                       ->  { projects: [{ projectId, hostId, root, name, chats }] }
project.remove  { projectId }             ->  { removed: true, chats }
```

* **`project.add` validates the path** (`is_dir`) and refuses a file, a typo or a folder a host cannot see
  with `` `<path>` is not a folder `` - which is why this is a method rather than a row the app writes. It
  **reuses** the row when the host already has that root, so pressing `Open folder` twice does not leave
  two rows for one directory.
* **`project.remove` unbinds its chats rather than deleting them** (`project_id` → `NULL`, then the row),
  and answers how many chats were unbound. A chat is a conversation and a folder is a place to have it:
  closing the folder must not throw the conversation away (principle P4). When chats *were* unbound the
  daemon pushes a toast saying so, because that is a thing the person should see.
* `session.open` gained an optional `projectId`, `session.update` gained one too (that is what "change this
  chat's folder" calls), and `session.list` reports `projectId` **and** `projectRoot` per row - so a folder
  survives a reload.
* `SessionOpened` and `SessionUpdated` carry the folder. A chat opened on a folder is right on its **first
  render**, with no second round trip to find out where it is.

### Added — the folder is where the engine runs

`Prompt` gained `project_root`, filled by `engine.start` from the session's project, and `cli.rs` starts the
child process **in** it. A chat with a folder runs there; a chat with none runs where the daemon is, which is
what every chat did before. A folder that has been moved or deleted since is **ignored rather than fatal**: a
turn that refuses to start is worse than one that runs where the daemon is.

`fs.search`, `git.status`, `git.diff`, `git.checkpoint`, `shell.run`, `checkpoint_create` and `rewind_apply`
all resolve the root the same way now (`root_for`): the envelope's own `root` / `projectRoot` first, because a
caller that names a directory means it, then **the session's folder**. That is the fix for the
checkpoint/rewind hole above - the app knows a chat's id, the daemon knows the folder, and a tool that had to
be told both was being asked to repeat a fact the daemon already held.

### Added — `Open folder` in the window

* The empty state is now spec section 7.13's **two** states: `No project` ("Open a folder to get started")
  when the window has no folder at all, and the familiar `No chat open` once it has one. The `Open folder`
  chip is the way out of the first, and opens the same native dialog the paperclip uses
  (`tauri-plugin-dialog`, `directory: true`) - a real folder chooser, not a text field.
* Opening a folder lands the person in a chat: the host's **empty** chat is re-pointed if it has one (the rule
  `+ New chat` has followed since 0.7.5, so opening a folder leaves no orphan row behind), otherwise a chat is
  opened **with** the folder. The tab opens and the caret goes to the prompt.
* The prompt toolbar gained a **folder chip** beside the model selector: `Working in SDC` (the last path
  segment) with the **full path as its tooltip**, or `No folder` for a chat that has none. It is absent when no
  chat is open, because then there is no chat whose folder could be shown. Pressing it opens the same dialog
  and re-points *this* chat.
* Three palette commands: `Open folder`, `Change folder` (on the active chat) and `Close this folder`
  (`project.remove`, then the workspace is re-read).

### Changed — the app's state gained a list

`AppState.projects` (`ProjectView`), folded by `withProjects` from `project.list`, and `SessionView` gained
`projectId` / `projectRoot`. Both are **reads folded as state patches**, the same contract `session.list` has
had since 0.6.1 - a folder is not something that *happens* to a chat the way a turn does, it is where the chat
is. `loadWorkspace` now loads both lists, so a window that reloads knows both.

### Verified

* `sdcd`: 144 unit tests plus 6 lifecycle tests against the real binary, including
  `a_folder_opened_on_a_chat_is_where_its_tools_look`: `project.add` (validate, reuse, refuse a file) →
  `session.open {projectId}` → the `SessionOpened` payload → `session.list` rows → **`fs.search` with nothing
  but a `sessionId`**, which finds a file in that folder. Three unit tests assert `Command::get_current_dir`:
  the engine runs in the chat's folder, a chat with no folder sets none at all, a folder that is gone is
  ignored. Clippy clean.
* `app`: 58 vitest cases (11 new: the folder arrives with the chat, a rename does not clear it, it survives
  the `session.list` replace, `project.list` is folded and replaced, `openFolderIn` re-points the empty chat /
  opens a new one / reports a refused path / will not land on a chat that has run a turn, `closeFolder`
  re-reads the workspace). Typecheck and lint at zero.
* `_verify/probe-folder.mjs`: in the running window - click `Open folder`, answer the **real** Windows folder
  dialog (`_verify/answer-folder-dialog.ps1`), then read the chip's label and tooltip, reload the window and
  read them again.

## [0.7.5] — "GUI file-picker nai", one chat per click, and a toast nobody could close

*"GUI file-picker nai. aita soho aro important kisu nai jeta vs code a thake. sudu local ashe. Aita fix
koro and bar bar new open korle onk chat open hoi and delete korle notification ashe middle a but
automatic jai nah ba remove ar option thake nah fix koro"* — three reports, three separate faults, none of
them about an engine.

### Fixed — the paperclip promised a file and opened nothing

`tauri-plugin-dialog` was a dependency, registered in `src-tauri/src/lib.rs`, and allowed by
`capabilities/default.json` (`dialog:default`). No line of the app ever called it: the two toolbar buttons
in the prompt box were `toast(...)` calls, so `Attach a file` and `Paste image` announced themselves and
did nothing.

`app/src/lib/picker.ts` is the picker now - `open()` with `multiple: true, directory: false` and, for the
image button, an image filter - and what it returns becomes a reference **inside the prompt**:

```
@H:\SDC\README.md
```

That is the shape an engine can act on, and it is a fact about the protocol rather than a taste: `engine.start`
carries the prompt and nothing else, so a path in the prompt travels to the daemon today while an attachment
chip would be decoration until attachments travel with the turn. The `1 file` chip beside the model selector
became the real count (`attached` was a `useState` with no setter), the caret goes back to the prompt box, and
the chip resets when the turn is sent. A browser tab has no filesystem to point at, so the fallback opens an
`<input type="file">` and returns names (`path === name` is how the caller can tell).

The image button also stopped saying `Paste image`: it opens a picker, so it says `Pick an image`.

### Fixed — `+ New chat` made a new chat every time

`newChatOnHost` called `session.open` on every click, so five clicks left five chats - four of them empty
rows to delete by hand. `emptySessionOn` (exported from `store/intents.ts`, and pure) decides first: the
host's existing chat with **no turn in the log for it** is the chat the click lands on, preferring the tab
the caret is already in. Only a host with nothing empty gets a new row.

### Fixed — a toast could not be closed, and its 3 seconds kept being pushed back

Two faults in the same stack:

* the close control was rendered **only next to an action chip**, so a message with no action (`Chat
  deleted`) could not be dismissed by hand at all - only waited out;
* the timer effect re-armed **every** toast whenever any toast appeared or left, so in a window with any
  traffic the oldest message's deadline moved for ever. Measured before the fix: two toasts, the older one
  still on screen after 3.4s.

Now every toast carries an ×, and the timers live in a map keyed by toast id: a toast gets one timer, a
toast that arrives later does not touch it, and the timer that fires removes its own entry.

### Added — a version you can check from inside the window

The number lives in five files, and the one place a person looks at it - Settings → About - had it typed
by hand: on this 0.7.5 build the dialog said `v0.4.4`. The three rows now come from what they describe
(`package.json` for the window, the event log's `HostStatus` for the daemon, `protocol/types.ts` for the
protocol), which is where the status bar's `v0.7.5 · sdcd 0.7.5` cell already read its two.

With that, a bump is one command and a check is one command:

```
node _verify/bump-version.mjs 0.7.6     # the five files that carry it
node _verify/version-report.mjs         # what every place says now; exit 1 when they disagree
```

`version-report.mjs` prints the five files, both lockfiles, the built daemon's `--version`, the window's
`VersionInfo` and every installer under `bundle/`, so "is my install actually the version I think?" is
answered from the tree rather than from memory; `probe-versions.mjs` answers it from the running window
instead, out of the status bar cell and the About rows. The recipe is in `docs/RELEASE.md`, including the
trap: `tauri build` fails with `Access is denied (os error 5)` while `sdc.exe` is still running, and the
installer that build leaves behind is the *previous* one - which is how a fix appears not to work.

Fixed in the same dialog: About's `This host` row read `Local · ` with a separator pointing at nothing.
`session.list` replaces the host rows and the `hosts` table has no `platform` column, so the one field the
list does not carry was being blanked - the same replace that `sdcd` already survived. `platform` survives
it now, pinned by a reducer test.

### Verified

`_verify/probe-075.mjs` runs all three in the built window. The dialog is answered from outside the page,
because it cannot be answered from inside it: Tauri freezes `window.__TAURI_INTERNALS__` (assigning to
`invoke` silently does nothing - a wrapper was measured never to run while `sdcp_status` answered anyway),
so `_verify/answer-dialog.ps1` finds the window with UI Automation and sends `WM_CHAR` to the file-name box
plus `WM_COMMAND(IDOK)` to the dialog. Against the 0.7.5 build:

```
new chat: rows before 16 -> 17, 17, 17
new chat toast: "Using the empty chat on local"
ok   the second and third clicks changed nothing
click: {"clicked":true,"before":"1 file"}
dialog: dialog: Open · answered: H:\SDC\README.md
prompt: "@H:\\SDC\\README.md " · chip: "2 files" · focused: true
ok   the paperclip was there and was clicked
ok   it opened the file dialog
ok   the dialog was answered with a path
ok   the prompt carries the picked path as a reference
ok   the chip counted one more file ("1 file" → "2 files")
ok   the caret went back to the prompt box
toast after delete: {"texts":["Chat deleted"],"closes":1}
3.2s later: []
two toasts: 2 · 3.4s after the first: 1 · after the ×: 0
ok   the delete toast carries a close ×
ok   it left on its own after its hold
ok   two toasts stack
ok   the older toast expired without waiting for the newer one
ok   the × closed the last one by hand

0.7.5: pass
```

`_verify/probe-streaming.mjs` still passes against this build (`the stream was live for 326ms before the
turn ended`), `_verify/probe-versions.mjs` passes against it too (`v0.7.5 · sdcd 0.7.5` in the status bar,
`v0.7.5 / v0.7.5 / 0.1` in About, and `Local · windows · x86_64` as the host row), and the suites are green:
`sdcd` **154 tests** with clippy clean under `-D warnings`, the window's `pnpm typecheck`, `pnpm lint` and
**47 vitest tests** (nine new: `lib/picker.test.ts` for the path shapes, `store/intents.test.ts` for the
empty-chat rule and the "does not ask the daemon" half of it, `store/reducer.test.ts` for the platform that
survives `session.list`).

Two things the user asked for are **not** in this build, and are named rather than half-built:

* **"sudu local ashe"** — the app still has one real host (`local`) and remote hosts are a *record* of a
  machine rather than a second daemon this window talks to. That is the SSH step, not this one.
* the **project (working-directory) browser** — the README's own next step, and the thing a VS Code user
  notices first: no folder is attached to a chat, so the engines run in whatever directory the daemon was
  started in. `fs.list` exists on the daemon and has no caller in the app yet.


## [0.7.4] — "live dakha jai nah, akbare answare disse"

*"suno ak ak kore issue solve kori. akhon claude and deepseek api kaj korse. But bisoy ta holo claude
code/cline/codex or others - a command dile jemon thinking ki korse sob kisu live dakha jai chat a,
aitate tamon kisui hosse nah, akbare answare disse."* — **the engine streamed and the daemon
collected.** `claude --include-partial-messages`, DeepSeek's SSE and Ollama's NDJSON were all arriving
a token at a time; `sdcd` held every one of them until the turn was over and then handed the window a
finished transcript.

### Fixed — `Engine::start` answered with a `Vec<EngineEvent>`

The contract itself was the defect: an adapter returned its whole stream, so `run_turn` could not push
anything before the engine had finished, and `TurnStarted` was followed by nothing for minutes. It now
takes a sink:

```rust
async fn start(&self, prompt: Prompt, sink: &EventSink);
```

`EventSink` is an `Arc` around one closure with no transport knowledge in it. `run_turn` hands in a
channel and consumes it while the engine runs (`EventSink::channel`), which keeps the two properties
the checkpoint rule needs — **order** (one producer, one consumer, so a `ToolCallStarted` cannot
overtake the `CheckpointSaved` written for it) and **one place that talks to the notifier**. A `Vec` is
still available where a whole stream in one piece is wanted (`Recorder`, `start_recording`).

### Fixed — every adapter parsed a finished batch

* **`cli.rs`** read stdout into a `Vec<String>`, parsed it after `child.wait()`, and only then answered.
  Each line is now parsed and pushed the moment `next_line` returns it (`push_stream_line`, which latches
  at the first `result`/`error`, exactly as `collect_stream` did). The "a CLI that never reached a
  result" rule is unchanged, and now runs against a stream that has already gone out.
* **`native_api`** read the whole HTTPS/HTTP body (`read_lines`) and then parsed it. `post_stream` now
  reads the response head, checks the status, and drains the body line by line, pushing each frame as it
  arrives (`push_sse_line`); a body that closes without `[DONE]` still ends the turn with `Done` rather
  than leaving it `running` for ever. The blocking call runs under `spawn_blocking`, so it no longer
  parks a runtime worker for the length of an answer. `parse_sse` stays as the collected form and is a
  fold over the same `parse_sse_line`.
* **`ollama`** read the whole body into a `String` and split it. It now streams over its own socket
  (`drain_chat`, `push_chat_line`), and its "not running" sentence belongs to the connect failure rather
  than to every failure.
* **The chunked framing is decoded** (`engines/body.rs`). A stream of unknown length is sent
  `Transfer-Encoding: chunked`, and reading such a body as raw lines gives size lines where frames
  should be — and a chunk boundary in the middle of a JSON object, which is a turn that stops
  mid-answer. Read without the framing decoded:

  ```
  1a            <- a chunk size, where a frame should be
  {"message":{"content":"He      <- the object, cut in half
  ```

### Fixed — the provider's own reasoning reached no one

`native_api` read `delta/thinking` (Anthropic) and nothing else, so a DeepSeek reasoner's
`reasoning_content` was dropped on the floor: the model thought and the window showed a model that does
not think. `parse_sse_line` now reads `/delta/thinking`, `/choices/0/delta/reasoning_content`,
`/delta/reasoning_content` and `/choices/0/delta/reasoning`, and they all become the same
`EngineEvent::Thinking` the thinking block of spec section 7.5 draws. An `error` frame *inside* a stream
(`{"type":"error","error":{…}}`) is now a failure with the provider's own sentence instead of a silence.

### Added — the window follows the stream (spec section 9.7)

`Pane` scrolls with the growth of the live turn while the reader is at the bottom, and stops the moment
they scroll up (24px of slack). The effect's dependency is the live turn's shape — its answer length,
its thinking length, its tool count — because `toTurns` builds a new array on every render.

### Verified

`sdcd`: **154 tests** (141 unit, 5 lifecycle, 6 VCR, 2 streaming) with clippy clean under
`-D warnings`, and
the new `tests/streaming.rs` measures the property over a real chunked socket: the test's server writes
one token, waits until that token is *in the sink*, and only then writes the rest — a collector cannot
pass it, because it would be waiting for the end of the body that only its own read can end.
`tests/vcr.rs` holds the live path and the collected path to the same events over all thirteen fixtures,
so a line cannot mean one thing on arrival and another in a batch.

`_verify/probe-streaming.mjs` is the end-to-end check. Against the real `claude_code` CLI, through the
real daemon, over SDCP:

```
+    197ms  -> engine.start answered {"turnId":"turn-1"}
+    198ms  TurnStarted
+   1912ms  TurnDelta        1 2 3 4
+   2409ms  TurnDelta         5 6 7 8
+   2410ms  TurnDelta         9 10 11 12
+   2410ms  TurnDelta         13 14 15 16
+   2410ms  TurnDelta         17 18 19 20
+   2507ms  TurnCompleted    Done

ok   the turn reached TurnCompleted
ok   engine.start answered first (+197ms)
ok   5 TurnDelta events arrived
ok   the first delta arrived at +1912ms
ok   the first delta beat TurnCompleted by 595ms
ok   the stream was live for 595ms before the turn ended
ok   the deltas were spread over 498ms, not one lump
ok   no delta was invented before the call answered

streaming: pass
```

The number that matters is `the stream was live for 595ms` — zero, before this change, for every
engine: a collected stream hands every delta over in the same tick as `TurnCompleted`, so that
difference was 0ms on every run.

The window: `pnpm typecheck`, `pnpm lint`, **38 vitest tests**, unchanged.


## [0.7.3] — the answer had no place to go

*"ami to oke sms korasi… kono response pelam nah, sudu done lakha aslo"* — **the engine answered and the
window threw the text away.** Not a provider, not a key, not a model: the turn stream could not draw an
answer at all.

### Fixed — `Turn` had no field for the answer

`panels/turns/types.ts` described a turn as `user`, `meta`, `thinking`, `tools`, `error`, `footer`, and
`live.ts`'s `toTurns` builds exactly that. So the text the reducer accumulates in `TurnView.text` - one
`TurnDelta` at a time, from any engine - had nowhere to go: `toTurns` could not pass it and `TurnStream`
had nothing to render. Every turn on every engine therefore read

```
You · Reply with exactly: BANANA-42
Balanced · claude_code · sonnet
Done · $0.0312 · 4.0s · 2 in · 141 out
```

with nothing between the third line and the fourth. The daemon's own log had the answer the whole time -
1804 `TurnDelta` events with real text for one turn, `141 out` in the totals - which is why every earlier
check "passed": the turn *ran*.

* `AnswerData` (`{ text, streaming }`) is part of `Turn` now, `toTurns` fills it from `TurnView.text`, and
  `AnswerBlock` draws it between the tool cards and the totals.
* The block keeps the answer's own line breaks (`whitespace-pre-wrap`): a CLI answer is written text with
  indented code in it, and this build has no markdown renderer to reflow it safely.
* A caret (`▍`) and the word `streaming…` appear only while deltas are still arriving.

`_verify/probe-answer.mjs` is the check that would have caught it: it reads the `[data-answer]` block's
text, so a pass is the engine's own words in the DOM rather than a substring of the prompt (which is what
the earlier probes were accidentally matching). Against `claude_code`:

```
open   opened
send   sent
+6000ms {"blocks":2,"streaming":0,"answer":"ANSWER BANANA-42"}
```

### Fixed — and one thing about this build's own process

Three `pnpm tauri:build` runs in a row had **failed** (`spawnSync rustc ENOENT`: `cargo` was not on that
shell's `PATH`) while I was reading the success of the *tests* as the success of the *package*. The window I
photographed was therefore an older bundle, which is how a fixed frontend looked like an unfixed one.
`_verify/` now checks the bundle the executable actually embeds:

```
exe bundle: index-B6Y6ua_X.js · dist bundle: index-B6Y6ua_X.js
```

### Verified

`sdcd`: 133 unit tests, 5 lifecycle, 5 VCR, clippy clean under `-D warnings`. The window: `pnpm typecheck`,
`pnpm lint`, **38 vitest tests** (six new: `panels/turns/live.test.ts` pins the answer mapping, including a
turn that has produced no text yet and one still streaming), the bundle smoke run, and the screenshot at
`_verify/shots/answer.png` - two turns, each with its `ANSWER` block above its totals.


## [0.7.2] — "chat e kisu likhle kaj hoy nah"

The report was one sentence: **Claude connects, but typing in the chat does nothing.** It was two
separate faults, and neither of them was Claude.

### Fixed — choosing a model in the Connect dialog did not choose anything

`chooseModel` wrote to the daemon (`models.select`) and **never touched the window's own model store**, and
`sendPrompt` reads the store. So `Use` on a row in the Connect dialog changed the daemon's setting, the row
said `In use`, and the chat kept running the model it already had - the person had picked a model and typed
into a chat that was still on another engine. It now writes both: the same four facts the model dropdown
sets (`engine`, `providerId`, `model`, `tier`), with the tier taken from the row when the catalogue listed
it. Proven by a live daemon turn and by `src/store/intents.test.ts`, which fails on the old behaviour.

### Fixed — a model from a provider's own list could not resolve to that provider

`native_api`'s endpoint table had three rows - `anthropic`, `openai`, `custom` - matched by the model id's
prefix, so every other provider the catalogue ships (DeepSeek, Groq, OpenRouter) matched nothing and fell
through to the `custom` loopback endpoint. The turn then failed with

```
native_api failed · No API key for custom. Connect it in the Provider Hub …
```

on a machine where DeepSeek was *connected*, with its key sitting under its own entry. Two changes:

* **the provider travels with the turn.** `engine.start` takes an optional `provider`, `Prompt` carries it,
  and `endpoint_for(model, provider)` uses it first - which is the only way to route a model id from a
  provider's **live** list, because this build's catalogue has never seen those ids (`deepseek-v4-pro`).
* **the catalogue resolves the rest.** For a caller that sends only a model id, the block that listed it
  decides: provider id, key entry (`sdc.provider.deepseek`, the one the Provider Hub wrote), protocol
  (`anthropic` → `messages` + `x-api-key`, anything else → `chat/completions` + Bearer) and the chat URL
  its `live` URL implies (`https://api.deepseek.com/v1/models` → `…/v1/chat/completions`).
  `api_model` now strips only a prefix that *is* a provider, so OpenRouter still receives
  `anthropic/claude-sonnet-4-5` and not a name it has never heard of.

Verified live, over SDCP, against the real provider:

```
engine.start { engine: native_api, model: deepseek-v4-pro, provider: deepseek }
  6ms     TurnStarted
  1010ms  TurnDelta "OK"
  1012ms  TurnCompleted  Done
```

`custom` is still the last resort, for a model id nothing claims.

### Verified

`sdcd`: 133 unit tests (four new on endpoint resolution), 5 lifecycle, 5 VCR, clippy clean under
`-D warnings`. The window: `pnpm typecheck`, `pnpm lint`, **32 vitest tests** (four new), the bundle smoke
run. `_verify/probe-native-turn.mjs` walks the UI for this exact case; `_verify/probe-turn.mjs` now takes a
model and a provider.


## [0.7.1] — the connection dialogs, re-drawn

Same daemon, same behaviour, one surface rebuilt. The report was a screenshot of the API-key dialog with
everything on it circled - unlabelled key box, `Save` and `Refresh` squeezed together with a footnote
wrapping between them, a tiny `MODELS` heading, and a `Use` button shaped exactly like every other button
on screen: *"koto useless and normal… button gulaw useless… sob gulatai aki"*. All of that was true.

### Changed — the dialog has a shape now

* **Header, sections, footer.** A provider tile, the name, a `connected` badge and one sentence; then
  sections with an uppercase name, their own controls and a note under them (`Credential`, `Models`); then
  the footer's single decision (`Save key`) beside the way out (`Close`). Nothing is decided twice:
  `Save key` is no longer next to `Refresh`, and `Refresh` moved up into the section it refreshes.
* **The key field is a field.** It has a label, a `Show`/`Hide` button, a `Paste` button, and a status
  badge that says `saved` or `not saved yet` - the old one was a bare `<input type="password">` with a
  placeholder and no label at all.
* **The model list is a list.** A name over its id, muted badges for tier, context, cost and where the row
  came from (`cached` / `live` / `bundled`), a filter box once there are more than five, and a row that is
  in use is tinted with `✓ In use` instead of wearing a disabled button.

### Fixed — a disabled button looked enabled

The button tones never carried a `disabled:` state, so `Save` on an empty key box was the same saturated
accent as `Save` with a key typed in. Nothing on screen said whether a press would do anything, which is
the honest reason the buttons read as useless. Every base class now dims and refuses the pointer, and the
sizes are named (`BTN_SM` 24px, `BTN` 28px, `BTN_LG` 34px) so a dialog can have hierarchy at all.

### Fixed — the same four-line paragraph over the window at every launch

`host.add` pushed the `ssh` probe's sentence as a `Toast` **as well as** onto the host's status line. Toasts
are events, events are in the log, and the first connection replays the log - so a machine that could not
be reached put the same paragraph (and, after three attempts, three of them) over the Provider Hub every
time the app started. Three changes, from the cause outwards:

* the daemon writes the probe's sentence to `HostStatus` only, where the card renders it;
* the window drops `Toast` events that arrive during its catch-up window - a replayed toast is not news;
* a toast's text clamps to two lines whatever it says, with the whole sentence on its `title`.

### Fixed — smaller lies in the same dialog

* `0K context` on a row whose catalogue has no context figure now reads `context not listed`.
* `Balanced` and `fast` were the wire's own spelling: tiers are title-cased for display, and a row's tier
  badge no longer disagrees with the tier the store keeps.
* A missing price prints `—` rather than nothing at all.
* The `MODELS` heading carries the count and the bundle's date (`2 rows · bundled 2026-09-20`), so the
  snapshot stops wrapping between two buttons.

### Verified

`sdcd`: 129 unit tests, 5 lifecycle, 5 VCR, clippy clean under `-D warnings`. The window: `pnpm typecheck`,
`pnpm lint`, 28 vitest tests, the bundle smoke run, and a real `SDC.exe` photographed over CDP - the
screenshot is in `_verify/shots/connect-deepseek.png` and is how the redesign was checked. `_verify/` gained
`shot-connect.mjs` (opens a provider's dialog and photographs it), `clean-probe-hosts.mjs` (removes the hosts
a probe added - the SSH probes had left a real row in a real database) and `bump-version.mjs`.



## [0.7.0] — the boxes people actually type in, and a VPS you can actually reach

Five reports, and four of them are the same kind of defect: a surface that looked finished and was not
wired to anything real. Everything in 0.6.4 is in this build.

### Fixed — the code box could not be pasted into

* **`Modal` stole focus once a second.** Its focus effect depended on `onClose`, and every caller passes
  an inline arrow, so the effect re-ran on *every render* - and the Connect dialog renders once a second
  while it polls a sign-in. Each render yanked focus to the dialog's first control, the URL field above
  the code field, which is exactly what "the box does something by itself and nothing can be pasted"
  describes. Focus now happens once, when the dialog opens, with the latest `onClose` held in a ref.
* **The dialog shifted under the pointer.** The CLI's output block grew a line at a time and moved the
  fields above it; it has a fixed height now and scrolls inside itself.
* **The code field takes focus once**, the moment the CLI is waiting for a code, keyed by login id.

### Fixed — the model menu above the chat box was dummy

`ENGINE_MODELS` was a hardcoded list of four invented names per engine (`sonnet`, `gpt-5`, `flash`,
`llama3.2`), and it was the *whole* model menu: it did not know what the user had connected, and
picking `Opus` did not reach the CLI. Now:

* the rows are the daemon's own catalogue (`models.list` - live from the provider, cached from its last
  answer, or from the shipped bundle), **grouped by provider and ordered with the connected ones first**;
* every row carries a readable `name` (`Claude Sonnet 4.5`, `Claude Opus · plan`, `DeepSeek Reasoner
  (R1)`, `GPT-5 Mini`), from the bundle where it has one and from a spelling rule where the provider
  sent only an id;
* a provider that is **not** connected is still listed, with a `Sign in to …` / `Add a key for …` row
  above its models, because the thing the user has to do should not be hidden;
* the engine group is gone: which engine runs a model is a property of the provider, so a row sets the
  engine and the model together;
* **and the model reaches the CLI**: `claude --model opus`, `codex exec -m …`, `gemini -p … -m …`.
  Measured: `claude -p --model haiku …` answers `"model":"claude-haiku-4-5-…"` in its own init line,
  and an unknown id fails in the provider's words (`[claude-code:unrecognized_model]`).

### Fixed — VPS connect, which did not exist

The report pasted `ssh -p 8443 deploy@203.0.113.10` - precisely what a person types into their
own terminal - and the daemon used the whole string as a hostname: `ssh` was asked for a machine called
`ssh`, and the port was never used. Then the honest-but-useless sentence appeared: *"…asks for a password
or a verification code, and SDC runs ssh without a terminal, so it cannot type it. Add your public key…"*.
Now:

* **the target is parsed** (`auth::remote::parse_target`): the `ssh` prefix, the `-p 8443`, `-p8443` and a
  trailing `-p 8443` all come out, and the port travels with every `ssh` call for that host. A bare
  hostname is refused with the sentence that says why, rather than guessing `root`;
* **the daemon has a terminal after all** - the PTY it already used for CLI sign-ins. Add the host with
  its password filled in and SDC runs `ssh <target> "<append my key>"`, answers the `password:` prompt,
  and drops the password: it is stored nowhere and appears in no sentence;
* **SDC owns a key**: `~/.ssh/sdc_ed25519`, made with the same `ssh-keygen -t ed25519 -N ""` a person
  would run, so it can be seen, used and revoked like any other key (delete one line from
  `authorized_keys`);
* a host that wants a **verification code** is the one case that cannot be automated, and it says so with
  the two ways forward instead of pretending - a one-time code is a second factor;
* the old refusal sentence now tells the user the thing that works: *add this host again with its
  password*.

Measured against the real VPS in the report: the probe reached the address **on port 8443** in 1.8s and
answered *"deploy@203.0.113.10 answered, but it asks for a password or a verification code. Add
this host again with its password filled in…"*, with the public-key generator verified separately.

### Fixed — the faint rows in every box

Two rows of decoration were removed from the prompt area, and the reason is the report: a faint
`Balanced · claude_code · sonnet` line sat inside the box under the Send button, and a row of tag-shaped
`@` / `/` / `⌘K` chips sat under the box. Both said things twice - the model line is what the selector
one row above already says, in the same words - and the chips advertised an `@` picker and a `/` command
list that do not exist.

### Fixed — a dialog with no close button

`ModalProps.bare` has documented "`true` renders the frame without a close button" since the frame was
written, and the button was never rendered: every dialog could only be closed with Escape or by clicking
the dimmed backdrop. There is an X in the frame's corner now. And a sign-in that **finishes closes
itself** after 2.6 seconds, with the toast and the flipped card left behind as the record.

### Verified

`sdcd`: **129 unit + 5 lifecycle + 5 VCR** tests (one `--ignored` test makes SDC's key, on purpose),
clippy clean under `-D warnings`. app: 28 vitest, typecheck/lint clean, bundle smoke 6/6. Live: a real
Claude turn and a real Codex turn through the daemon, a real `host.add` against the VPS from the report,
and `claude --model haiku` proving the chosen model reaches the CLI.

---


## [0.6.4] — the release gate stops depending on the internet

0.6.3's `Checks` step failed on one runner and passed on three others **for the same commit**, which
means the gate itself was the defect: a test of mine reached the real `api.groq.com` to prove that "a
refresh that failed keeps the list", so a runner with a different network path decided whether a release
could be published. Everything in the 0.6.3 entry below is in this build.

### Fixed — a flaky gate, and the defect it was standing on

* The refresh test now drives a **loopback server** that answers `401` with a provider-shaped body
  (`models::list_blocks` exists so a test can supply the provider blocks), so the check is deterministic
  and needs no network at all.
* Writing it found a real defect next door: the `http://` path in `providers::models` **ignored the
  status code**. A local OpenAI-compatible endpoint that answers `401` - anything behind a password -
  was reported as *"the endpoint answered without a model list"* instead of the body's own sentence.
  Both transports now report a rejection through the same `rejection()`, so the provider's words reach
  the user whichever one carried them.

### Verified

`sdcd`: **119 unit + 5 lifecycle + 5 VCR** tests, clippy clean under `-D warnings`; the failing test of
0.6.3 is deleted, not skipped - its coverage moved to two hermetic ones.

---


## [0.6.3] — the chat answers, and a sign-in finishes

Four defects, every one of them found by driving the installed CLIs *from* the built app rather than by
reading the adapters. 0.6.2 was replaced by this one the same day; the pipeline keeps exactly one
release.

### Fixed — every chat turn came back empty

Two of the three were the same mistake: a fact about the session, guessed from the wrong place.

* **The CLI invocations did not exist.** `claude` was run with `--include-partial-messages` alone, and
  Claude Code answers that with
  `Error: --include-partial-messages requires --print and --output-format=stream-json.` on **stderr** -
  which the adapter sent to `Stdio::null()`. Exit 1, empty stdout, and the turn ended with "the
  engine's stream ended without a result". Codex was further off: `--json --quiet` is not a flag of
  that CLI at all (`error: unexpected argument '--json' found`), and Gemini's `--output json` is not
  either (`Unknown argument: output`). The measured, working forms are now what the daemon runs:

  ```text
  claude   -p --output-format stream-json --include-partial-messages --verbose
  codex    exec --json --skip-git-repo-check -
  gemini   -p <prompt> --output-format stream-json --skip-trust
  ```

  Gemini's prompt is an *argument* (its headless mode reads stdin as the answer to its own questions),
  which is why the spec carries a `PromptPlacement` rather than assuming a pipe.
* **The JSON parser knew an invented shape.** `parse_stream_line` was written against
  `{"type":"assistant_text","text":"hello"}` and `{"type":"result","summary":"Done"}` - shapes no CLI
  has ever sent - so even a correct invocation parsed to zero events. It now reads Claude's
  `stream_event`/`content_block_delta`, Codex's `item.completed`/`agent_message` and `turn.completed`,
  and Gemini's `message`/`result`, all copied from captures taken with `_verify/cli-capture.mjs`.
  Claude's `assistant` line is deliberately dropped: with partial messages the deltas already carried
  the text, and emitting both would print every answer twice.
* **Silence is no longer possible.** `stderr` is captured, and a turn that did not reach `Done` ends
  with the CLI's own first line:

  ```text
  `claude` said: Error: --include-partial-messages requires --print and --output-format=stream-json.
  ```
* **The model was guessed.** `native_api` read the model out of the **prompt text** and `ollama` out of
  the **first history message**, so an API key could not work: a chat on an Anthropic model went to the
  loopback endpoint (`127.0.0.1:8080`) unless the user happened to type the provider's name first. The
  model now travels in the `Prompt`, resolved once by `engine_start`, and the registry's
  `anthropic/claude-sonnet-4-5` becomes `claude-sonnet-4-5` on the wire - which is what
  `api.anthropic.com` knows.

Measured after the fix, through the daemon (`_verify/probe-turn.mjs`):

```text
claude_code  TurnDelta "O" · TurnDelta "K" · TurnCompleted        text: "OK"
codex        TurnDelta "OK" · TurnCompleted                       text: "OK"
native_api   engine.start{ native_api, model: anthropic/claude-sonnet-4-5 }
             → "No API key for anthropic …"    (before: "No API key for custom")
```

### Fixed — the sign-in could not finish, and a success stayed invisible

* **The authorize page was mistaken for a code.** Claude's page is
  `…/cai/oauth/authorize?code=true&client_id=…`, so a user who pasted the link SDC itself showed them
  handed the CLI the literal word `true` - and the provider answered
  `Login failed: Request failed with status code 400`, which is the screenshot that reported this. A
  pasted value that is the authorize page is now refused with the sentence that says what to paste
  instead; a real callback address still yields its code, and a value shorter than sixteen characters
  behind `code=` is no longer treated as one.
* **A finished sign-in is announced.** Only `cli.login.code` ever pushed `connected`, so a CLI that
  completes on its own - Claude Code opens the browser itself and finishes when the page is approved -
  left the card saying `connecting` under a login that had worked; reported as "even the one that
  succeeded isn't shown". `cli.login.status` now pushes `ProviderStatus{connected}` on the poll that
  first sees the CLI's success line, and the dialog says so with a toast.

### Changed — the sign-in dialog, re-drawn

The screenshot showed a code box and a `Submit code` sitting under a CLI that had already printed
`Login failed`. Now the state line is coloured by what happened (green signed in, amber waiting, red
stopped), the paste boxes are drawn **only** while the CLI is still waiting for a code, `Submit code`
is disabled until there is something to submit, the page and the code have their own labelled boxes
with the sentence that says which is which, and a stopped login offers `Try again` rather than a field
that cannot answer.

### Verified

`sdcd`: **118 unit + 5 lifecycle + 5 VCR** tests, 2 live checks behind `--ignored`, clippy clean under
`-D warnings`. A real `claude` turn and a real `codex` turn through the daemon (above), and a real
`native_api` turn proving the model now selects the endpoint. The window: `pnpm typecheck`, `pnpm
lint`, 28 vitest tests, the bundle smoke run.

---


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

