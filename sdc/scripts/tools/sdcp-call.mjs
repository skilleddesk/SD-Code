/**
 * One SDCP call from the command line - what `sdc/scripts/tools/sdcp-smoke.mjs` does for thirty of them at once.
 *
 *   node sdc/scripts/tools/sdcp-call.mjs session.list
 *   node sdc/scripts/tools/sdcp-call.mjs project.add '{ "root": "H:\\SDC" }'
 *   node sdc/scripts/tools/sdcp-call.mjs fs.list '{ "sessionId": "n3" }' 7811
 *
 * The port defaults to the daemon's usual 7811. A probe that fails in the *window* can be asked what the
 * daemon actually holds, which is the difference between "the UI is wrong" and "the data is wrong" - and it
 * is how the 0.7.7 file-tree probe's first failure was diagnosed (a chat that had vanished from the sidebar
 * was still, or not, in the database).
 */
import net from 'node:net';

const method = process.argv[2];
const params = process.argv[3] === undefined ? {} : JSON.parse(process.argv[3]);
const port = Number(process.argv[4] ?? 7811);

if (method === undefined) {
  console.error('usage: node sdc/scripts/tools/sdcp-call.mjs <method> [params-json] [port]');
  process.exit(2);
}

const socket = net.connect(port, '127.0.0.1');
let buffer = '';

socket.on('connect', () => {
  socket.write(`${JSON.stringify({ v: '0.1', id: 'cli-1', method, params })}\n`);
});

socket.on('data', (chunk) => {
  buffer += chunk.toString();

  for (const line of buffer.split('\n')) {
    if (line.trim() === '') {
      continue;
    }

    let message;

    try {
      message = JSON.parse(line);
    } catch {
      continue;
    }

    /* Only the answer to *this* call: everything else is a notification. */
    if (message.id === 'cli-1') {
      console.log(JSON.stringify(message, null, 2));
      socket.end();
    }
  }

  buffer = buffer.slice(buffer.lastIndexOf('\n') + 1);
});

socket.on('error', (error) => {
  console.error(`could not reach sdcd on 127.0.0.1:${port}: ${error.message}`);
  process.exit(1);
});

socket.setTimeout(Number(process.env.SDCP_TIMEOUT ?? 8000), () => {
  console.error('sdcd did not answer within 60s');
  process.exit(1);
});
