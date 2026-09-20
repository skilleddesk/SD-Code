# sdcd VCR fixtures (master spec §11.6)

One file per recorded conversation — **thirteen** in all: twelve ordinary turns plus
`native-13-user-pastes-screenshot.jsonl`, which is the vision path, a turn that starts from a pasted
screenshot rather than a typed prompt.

The first line of every file is the expectation:

```json
{"type":"vcr","engine":"claude_code","kinds":["Delta","Done"]}
```

The rest is the raw stream the adapter would have seen, in that engine's dialect:

| file prefix  | dialect                             | parser the test replays it with      |
| ------------ | ----------------------------------- | ------------------------------------ |
| `claude-`    | JSON lines                          | `engines::parse_stream_line`         |
| `codex-`     | JSON lines                          | `engines::parse_stream_line`         |
| `gemini-`    | JSON lines                          | `engines::parse_stream_line`         |
| `ollama-`    | NDJSON from `/api/chat`             | `engines::ollama::parse_chat_line`   |
| `native-`    | Server-Sent Events                  | `engines::native_api::parse_sse`     |

`tests/vcr.rs` replays all thirteen and asserts the event kinds match the expectation, and that every
stream ends at a terminal event (`Done` or `Failed`). That is the acceptance item "each engine's
fixture replay produces identical UI events to a live run", in testable form: the fixture and the
parser are one contract, so an adapter that starts emitting a new event — or stops emitting one —
fails the build until the fixture is regenerated deliberately.

There is one negative control too (`a_changed_stream_shows_up_as_a_changed_replay`), because a
fixture-replay test that cannot fail is not a test.

## Regenerating

`generate.mjs` sits next to these fixtures, so the whole thing travels with a clone:

```bash
node sdc/sdcd/tests/vcr/generate.mjs
```

Changing a fixture by hand is fine — the file *is* the record — but if a conversation's stream
changes, the `kinds` line on its first line has to change with it, and that edit is the conversation
about whether the change was wanted.
