# SDC as an MCP server (Phase 4.9) / SDC-কে MCP সার্ভার হিসেবে ব্যবহার

`sdcd mcp` makes SDC's safety layer available to **Claude Code, Cursor, Codex** and any other MCP client. It speaks the Model Context Protocol
(JSON-RPC 2.0, one message per line, version 2024-11-05) on stdin/stdout. **No tool calls an AI model.**

| Tool | Does | Changes anything? |
|---|---|---|
| `sdc_secret_scan` `{path}` | file, line and kind of every secret in the folder - never the value | no |
| `sdc_policy_check` `{path, action: read\|write\|run, target}` | `allow` / `ask` / `deny` from the folder's `.sdc/policy.toml`, with the reason | no |
| `sdc_verify` `{path}` | the project's own checks (typecheck, lint, tests, build) in SDC's sandbox | no |
| `sdc_checkpoint` `{path, title?}` / `sdc_checkpoints` `{path}` | Time Machine checkpoint / the list | adds a checkpoint |
| `sdc_restore` `{path, id, confirm}` | folder back to a checkpoint. **Needs `confirm: true`**; a safety checkpoint is taken first, so a restore can be undone | yes |
| `sdc_vps_run` `{host, command, confirm?}` | a command on a server connected in SDC (SDC must be running). Read-only commands run; anything else needs `confirm: true`; `rm -rf /`, `mkfs`, `dd`, `shutdown`, `DROP DATABASE`, fork bombs are **always refused** | only with confirm |

The folder must be a project folder (not `/`, not the home folder itself). A tool that wants a person's approval says so in its answer; the
agent is expected to ask you, then call again with `confirm: true`.

## Setup

First put `sdcd` where the client can run it. In the installed app it is inside the application bundle; the simplest is to copy or link it
(`/Applications/SDC.app/Contents/MacOS/sdcd` on macOS; the install folder on Windows; `sdcd` on `PATH` on Linux).

**Claude Code**

```sh
claude mcp add sdc -- /Applications/SDC.app/Contents/MacOS/sdcd mcp
```

**Cursor** - `~/.cursor/mcp.json` (or the project's `.cursor/mcp.json`):

```json
{ "mcpServers": { "sdc": { "command": "/Applications/SDC.app/Contents/MacOS/sdcd", "args": ["mcp"] } } }
```

**Codex** - `~/.codex/config.toml`:

```toml
[mcp_servers.sdc]
command = "/Applications/SDC.app/Contents/MacOS/sdcd"
args = ["mcp"]
```

`sdc_vps_run` talks to the SDC app on `127.0.0.1:7811`; add `"--port", "NNNN"` to the arguments if you run the daemon on another port. All
other tools work without the app running.

## Check it by hand

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
              '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | sdcd mcp
```

## What is not proven
Tested: the protocol handshake, all seven tools against real folders (the restore round trip uses real git), the policy answers, and the
server-command rules against a mock daemon (`cargo test --lib mcp::`). Not tested here: a real Claude Code, Cursor or Codex session, and
`sdc_vps_run` against a real server (OWNER-TO-RUN).
