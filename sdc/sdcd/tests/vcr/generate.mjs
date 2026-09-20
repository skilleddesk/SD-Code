// Regenerates the VCR fixtures (master spec §11.6) in this directory:
//
//   node sdc/sdcd/tests/vcr/generate.mjs        (from the repository root, or from anywhere)
//
// Twelve conversations plus "User pastes screenshot" (the vision path), each in the dialect of the
// adapter it belongs to. Changing a fixture by hand is fine - the file *is* the record - but this is
// what keeps the thirteen consistent with the event catalogue.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const out = import.meta.dirname;

mkdirSync(out, { recursive: true });

const conversation = (kinds, engine, body) => [
  JSON.stringify({ type: 'vcr', engine, kinds }),
  ...body,
];

const text = (value) => JSON.stringify({ type: 'assistant_text', text: value });
const thinking = (value) => JSON.stringify({ type: 'thinking', delta: value });
const tool = (id, name, target, which = 'edit') =>
  JSON.stringify({ type: 'tool_use', id, tool: which, name, target });
const toolDone = (id, meta) => JSON.stringify({ type: 'tool_done', id, meta, ok: true });
const result = (summary, meta, pass = true) => JSON.stringify({ type: 'result', summary, meta, pass });

const conversations = [
  ['claude-01-add-rate-limiting.jsonl', ['Thinking', 'Delta', 'Delta', 'ToolStarted', 'ToolCompleted', 'Delta', 'Done'], 'claude_code', [
    thinking('Reading src/auth.ts first.'),
    text('Added express-rate-limit '),
    text('to the login route.'),
    tool('c1', 'Edit', 'src/auth.ts'),
    toolDone('c1', 'done · +18 −2'),
    text('Run the tests next.'),
    result('Done', '1m 12s · 12,400 tokens · $0.16'),
  ]],
  ['claude-02-fix-login-bug.jsonl', ['Delta', 'ToolStarted', 'ToolCompleted', 'Delta', 'Done'], 'claude_code', [
    text('The session cookie was not set.'),
    tool('c1', 'Read', 'src/login.tsx', 'read'),
    toolDone('c1', 'done · 42 ln'),
    text('Fixed the SameSite flag.'),
    result('Done', '48s · 3,100 tokens · $0.04'),
  ]],
  ['claude-03-refactor-auth.jsonl', ['Delta', 'Done'], 'claude_code', [
    text('Split auth.ts into auth/ and session/.'),
    result('Done', '2m 03s · 21,000 tokens · $0.31'),
  ]],
  ['claude-04-fix-deploy-script.jsonl', ['Delta', 'ToolStarted', 'ToolCompleted', 'Failed'], 'claude_code', [
    text('Running the deploy script.'),
    tool('c1', 'Run', 'bash deploy.sh', 'run'),
    toolDone('c1', 'done · exit 0'),
    JSON.stringify({ type: 'error', message: 'scp: /var/www: Permission denied' }),
  ]],
  ['claude-05-log-aggregation.jsonl', ['Thinking', 'Delta', 'Done'], 'claude_code', [
    thinking('Vector over Filebeat: less to configure.'),
    text('Added a vector.toml pipeline.'),
    result('Done', '35s · 2,400 tokens · $0.03'),
  ]],
  ['claude-06-update-readme.jsonl', ['Delta', 'Done'], 'claude_code', [
    text('Rewrote the install section.'),
    result('Done', '22s · 1,100 tokens · $0.01'),
  ]],

  ['codex-07-offline-queue.jsonl', ['Delta', 'ToolStarted', 'ToolCompleted', 'Delta', 'Done'], 'codex', [
    JSON.stringify({ type: 'text', text: 'Queued writes while offline. ' }),
    JSON.stringify({ type: 'tool_call', id: 'x1', tool: 'edit', name: 'Edit', target: 'src/queue.ts' }),
    JSON.stringify({ type: 'tool_done', id: 'x1', meta: 'done · +64 −3', ok: true }),
    JSON.stringify({ type: 'text', text: 'Flush on reconnect.' }),
    JSON.stringify({ type: 'result', summary: 'Done', meta: '1m 04s', pass: true }),
  ]],
  ['codex-08-tighten-types.jsonl', ['Delta', 'Done'], 'codex', [
    JSON.stringify({ type: 'text', text: 'Turned on strict null checks.' }),
    JSON.stringify({ type: 'result', summary: 'Done', meta: '58s', pass: true }),
  ]],
  ['codex-09-failing-test.jsonl', ['Delta', 'Failed'], 'codex', [
    JSON.stringify({ type: 'text', text: 'Trying the fix.' }),
    JSON.stringify({ type: 'error', message: 'AssertionError: expected 6 to be 5' }),
  ]],

  ['gemini-10-compare-models.jsonl', ['Delta', 'Done'], 'gemini', [
    JSON.stringify({ type: 'text', text: 'Sonnet is cheaper per task here.' }),
    JSON.stringify({ type: 'result', summary: 'Done', meta: '40s', pass: true }),
  ]],
  ['gemini-11-port-to-ts.jsonl', ['Thinking', 'Delta', 'Done'], 'gemini', [
    JSON.stringify({ type: 'reasoning', delta: 'The JS files are small enough.' }),
    JSON.stringify({ type: 'text', text: 'Renamed 12 files to .ts.' }),
    JSON.stringify({ type: 'result', summary: 'Done', meta: '1m 20s', pass: true }),
  ]],

  ['ollama-12-local-summary.jsonl', ['Delta', 'Delta', 'Done'], 'ollama', [
    JSON.stringify({ message: { content: 'Rate limiting is ' }, done: false }),
    JSON.stringify({ message: { content: 'the answer.' }, done: true, eval_count: 12 }),
  ]],

  /* The vision path: the user pastes a screenshot, so the turn starts from an image rather than a
     typed prompt. The SSE dialect carries it, which is what the native adapter parses. */
  ['native-13-user-pastes-screenshot.jsonl', ['Delta', 'Thinking', 'Delta', 'Done'], 'native_api', [
    'event: message_start',
    'data: {"delta":{"text":"I can see the error toast "}}',
    'data: {"delta":{"thinking":"The screenshot shows a 500 from /login."}}',
    'data: {"delta":{"text":"in your screenshot."}}',
    'data: [DONE]',
  ]],
];

for (const [name, kinds, engine, body] of conversations) {
  writeFileSync(join(out, name), [...conversation(kinds, engine, body), ''].join('\n'), 'utf8');
}

console.log(`wrote ${conversations.length} fixtures to ${out}`);

// Run from this directory:  node generate.mjs
// (its output directory is this one: sdc/sdcd/tests/vcr)
