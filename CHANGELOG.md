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

The report pasted `ssh -p 8443 mehedi105117@109.199.108.216` - precisely what a person types into their
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
answered *"mehedi105117@109.199.108.216 answered, but it asks for a password or a verification code. Add
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

