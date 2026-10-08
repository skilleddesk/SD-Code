import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

/* One live round trip over TLS: a deliberately bogus key against the provider's own model list.
   A `401` with the provider's sentence proves the transport works - it reached the endpoint, presented the
   key and read the answer - without needing a key anybody would pay for. */
const method = process.argv[2] ?? 'provider.test';
const params = process.argv[3] ?? JSON.stringify({ id: 'anthropic-api', key: 'sk-ant-bogus-key-for-a-live-401' });
const port = process.argv[4] ?? '7833';

const answer = spawnSync('node', [fileURLToPath(new URL('./sdcp-call.mjs', import.meta.url)), method, params, port], { encoding: 'utf8' });

console.log(answer.stdout || answer.stderr);
