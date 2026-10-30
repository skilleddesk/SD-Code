# SDC - rules that must not change

These are behaviours the owner asked for explicitly, more than once. Each has tests that pin it down. If a test
named below fails after a change, the change is wrong - fix the change, not the test. Do not remove, weaken or
"simplify" any of these without the owner saying so in writing.

## 1. A message sent while a turn runs goes into that turn - for every AI

When the person sends a prompt while a turn is still working, the turn reads it, adjusts, and finishes with it.
It never waits in a queue for the turn to end. This holds for **every** engine: SDC's agent with any API
(DeepSeek, OpenAI, Anthropic, Gemini API, local models...), Claude Code, Codex, Gemini CLI, Ollama, and any engine
or CLI added in the future.

- Every turn runs through `engines::steer::drive` (`sdcd/src/sdcp/methods.rs`, `run_turn`). Never call
  `engine.start` directly there.
- An engine that can read messages between its own steps calls `steer::go_live` and reads them with
  `steer::take` (the agent loop; Claude Code via `--input-format stream-json`, `engines/cli.rs` `Steering`).
- Every other engine is stopped the moment a message arrives and the same turn goes on with it. A new engine gets
  this for free; its stop check should also read `steer::interrupting(turn_id)`.
- Tests: `engines::steer::tests::every_*`, `engines::cli::tests::a_message_sent_mid_turn_goes_into_the_running_cli`,
  `engines::cli::tests::a_result_before_the_message_was_read_does_not_end_the_turn`.

## 2. Git works in any project folder

Every `git` SDC runs - locally and on a server - trusts the folder the person opened (`safe.directory=*`), so
"detected dubious ownership" never appears (exFAT/FAT drives, folders owned by another account, `www-data` on a
VPS). See `host::env_path::trust_project_folders`, `github.rs` `TRUST_FOLDER`, `ssh::ops::TRUST`/`TRUST_ENV`.
Test: `host::env_path::tests::git_trusts_the_project_folder_after_any_config_already_given`.

## 2b. Push is "what I worked on", from any folder

The person opens any folder, picks a repository, presses Push - and gets exactly what `git add`, `commit`, `push`
by hand would give, with no errors to understand (`sdcd/src/github.rs`, `push_steps`):

- A folder with no commit, pushed to a repository that already has files, takes that repository's history first
  (`fetch` + `reset`, no file in the folder changes) and lands as one commit on top - never a force push, never
  unrelated histories. Files only GitHub has stay (`skip-worktree`), on this push and the next.
- Never sent: package stores and build output (`NEVER_SENT`), junk and key files (`NEVER_SENT_FILES`), a file holding
  a key (`KEY_PATTERNS`, and `KEY_ASSIGNMENT` in text/config files), a repository inside the folder. The window says
  what was kept back and why.
- A git lock held by another git is waited for, not shown as an error; `status` never takes the lock.
- Test: `github::tests::a_folder_opened_anywhere_is_pushed_into_a_repository_that_already_has_files` (real git).

## 3. The update notice

Top right, closes by itself after a while, has a cross, and comes back every time SDC opens until SDC is updated
(no "skip this version"). `app/src/overlays/UpdateNotice.tsx`.

## 4. A release is not done until `latest.json` is signed

After pushing a `vX.Y.Z` tag and the `release` workflow finishes, run `node sdc/scripts/sign-release.mjs vX.Y.Z`
from this folder; otherwise the in-app Update button falls back to a GitHub link.

## Checks

- Daemon: `cd sdc/sdcd && cargo test --lib`
- App: `cd sdc/app && npm run typecheck && npx vitest run`
