/**
 * A/B speed benchmark of the SDC Agent against a real model, through a real daemon (0.20).
 *
 *   node sdc/scripts/tools/bench-agent.mjs <port> <label> <task> [provider] [model]
 *
 * It builds a small throw-away project in the system temp folder (git-initialised, one failing test), opens
 * a chat on it, runs ONE turn of the SDC Agent with the task's prompt and prints one JSON line: wall time,
 * time to the first visible output, model steps, tool calls, tokens, whether the result is right.
 *
 * Run it against two daemons started with `--port <n> --database <temp file>` (an old build and a new one)
 * to compare them on the same task. It never touches the person's own daemon, folders or database - only
 * the connected provider's key, which the daemon reads from the keychain itself.
 *
 * Tasks: question | fix | feature
 */
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const [, , portArg, label, task, provider = 'qwen', model = 'deepseek-v4.1-flash'] = process.argv;
const port = Number(portArg);

const PROMPTS = {
  question: 'why does add(2, 3) return -1 in this project?',
  fix: 'add(2,3) dile -1 ashe keno? thik kore dao jate npm test pass kore',
  feature:
    'add multiply(a, b) and divide(a, b) to src/math.js (divide by zero must throw an Error), export them from src/index.js, add tests for both in test.js, and list them in README.md',
};

if (!Number.isInteger(port) || label === undefined || PROMPTS[task] === undefined) {
  console.error('usage: node bench-agent.mjs <port> <label> <question|fix|feature> [provider] [model]');
  process.exit(2);
}

const run = (cwd, file, args) => execFileSync(file, args, { cwd, stdio: 'pipe' });

function makeProject() {
  const root = mkdtempSync(join(tmpdir(), `sdc-bench-${label}-`));

  mkdirSync(join(root, 'src'));
  writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'demo', version: '1.0.0', scripts: { test: 'node test.js' } }, null, 2) + '\n');
  writeFileSync(join(root, 'src', 'math.js'), 'function add(a, b) {\n  return a - b;\n}\n\nfunction sub(a, b) {\n  return a - b;\n}\n\nmodule.exports = { add, sub };\n');
  writeFileSync(join(root, 'src', 'index.js'), "module.exports = require('./math');\n");
  writeFileSync(
    join(root, 'test.js'),
    "const assert = require('assert');\nconst { add, sub } = require('./src');\n\nassert.strictEqual(add(2, 3), 5, 'add');\nassert.strictEqual(sub(5, 2), 3, 'sub');\nconsole.log('ok');\n",
  );
  writeFileSync(join(root, 'README.md'), '# demo\n\nSmall math helpers.\n\n## API\n\n- add(a, b)\n- sub(a, b)\n');
  run(root, 'git', ['init', '-q']);
  run(root, 'git', ['add', '.']);
  run(root, 'git', ['-c', 'user.name=bench', '-c', 'user.email=b@b', 'commit', '-q', '-m', 'start']);

  return root;
}

const root = makeProject();
const socket = net.connect(port, '127.0.0.1');
const pending = new Map();
const events = [];
let buffer = '';
let counter = 0;
const t0 = performance.now();

socket.on('data', (chunk) => {
  buffer += chunk.toString();

  const lines = buffer.split('\n');

  buffer = lines.pop() ?? '';

  for (const line of lines) {
    if (line.trim() === '') continue;

    let message;

    try {
      message = JSON.parse(line);
    } catch {
      continue;
    }

    if (message.id !== undefined && pending.has(message.id)) {
      pending.get(message.id)(message);
      pending.delete(message.id);
      continue;
    }

    const event = message.event ?? message.params?.event ?? message.params ?? null;

    if (event !== null && typeof event.type === 'string') {
      events.push({ at: performance.now() - t0, type: event.type, event });
    }
  }
});

const call = (method, params) =>
  new Promise((resolve, reject) => {
    const id = `b${++counter}`;

    pending.set(id, (message) => (message.error ? reject(new Error(`${method}: ${JSON.stringify(message.error)}`)) : resolve(message.result)));
    socket.write(`${JSON.stringify({ v: '0.1', id, method, params })}\n`);
  });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

await new Promise((resolve, reject) => {
  socket.once('connect', resolve);
  socket.once('error', reject);
});

await call('event.subscribe', {});

const project = await call('project.add', { root });
const projectId = project.projectId ?? project.project?.id ?? project.id;
const session = await call('session.open', { hostId: 'local', title: `bench ${label}`, projectId });
const sessionId = session.sessionId ?? session.session?.id ?? session.id;
const started = performance.now();
const turn = await call('engine.start', { sessionId, engine: 'native_api', provider, model, prompt: PROMPTS[task], agent: true, autonomy: 'auto', tier: 'Balanced' });
const turnId = turn.turnId;
const deadline = started + 300_000;
let ended = null;

while (performance.now() < deadline) {
  ended = events.find((entry) => (entry.type === 'TurnCompleted' || entry.type === 'ErrorRaised') && (entry.event.turnId === turnId || entry.event.turnId == null));

  if (ended !== undefined) break;

  await sleep(200);
}

const mine = events.filter((entry) => entry.event.turnId === turnId || entry.event.turnId == null);
const first = (types) => mine.find((entry) => types.includes(entry.type));
const firstOutput = first(['TurnDelta', 'ThinkingDelta', 'ToolCallDrafting', 'ToolCallStarted']);
const firstWords = first(['TurnDelta']);
const tools = mine.filter((entry) => entry.type === 'ToolCallStarted').map((entry) => entry.event.name);
const text = mine.filter((entry) => entry.type === 'TurnDelta').map((entry) => entry.event.delta).join('');
const completed = mine.find((entry) => entry.type === 'TurnCompleted');
const meta = completed?.event.meta ?? '';
const steps = Number(/(\d+) steps?/.exec(meta)?.[1] ?? 0);
const tokensIn = Number(/([\d.]+)k? in/.exec(meta)?.[1] ?? 0) * (/k in/.test(meta) ? 1000 : 1);
const failure = mine.find((entry) => entry.type === 'ErrorRaised');

/* Is the result right? */
let correct = null;

try {
  if (task === 'question') {
    correct = /a\s*-\s*b|subtract|minus|\bsub/i.test(text);
  } else if (task === 'fix') {
    run(root, 'node', ['test.js']);
    correct = true;
  } else {
    run(root, 'node', ['test.js']);

    const math = readFileSync(join(root, 'src', 'math.js'), 'utf8');
    const readme = readFileSync(join(root, 'README.md'), 'utf8');
    const tests = readFileSync(join(root, 'test.js'), 'utf8');
    const lib = join(root, 'src', 'index.js').replace(/\\/g, '/');
    const thrown = execFileSync('node', ['-e', `try { require('${lib}').divide(1, 0); console.log('no') } catch (e) { console.log('throws') }`], { encoding: 'utf8' }).trim();

    correct = /multiply/.test(math) && /divide/.test(math) && /multiply/.test(readme) && /divide/.test(readme) && /multiply/.test(tests) && /divide/.test(tests) && thrown === 'throws';
  }
} catch {
  correct = false;
}

console.log(
  JSON.stringify({
    label,
    task,
    model: `${provider}/${model}`,
    wallSeconds: Number((((ended?.at ?? performance.now() - t0) - (started - t0)) / 1000).toFixed(1)),
    firstOutputSeconds: firstOutput === undefined ? null : Number(((firstOutput.at - (started - t0)) / 1000).toFixed(1)),
    firstWordsSeconds: firstWords === undefined ? null : Number(((firstWords.at - (started - t0)) / 1000).toFixed(1)),
    steps,
    toolCalls: tools.length,
    tools: tools.join(','),
    meta,
    answerChars: text.length,
    correct,
    error: failure === undefined ? null : String(failure.event.title ?? failure.event.explanation ?? '').slice(0, 160),
    project: root,
  }),
);

socket.end();
process.exit(0);
