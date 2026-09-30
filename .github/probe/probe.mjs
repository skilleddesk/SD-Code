// Mac probe: what the installed app's daemon does when "Sign in" is pressed.
import net from 'node:net';

const port = 7811;
let seq = 0;

function call(method, params = {}, timeout = 60000) {
  return new Promise((resolve) => {
    const id = `p-${++seq}`;
    const socket = net.connect(port, '127.0.0.1');
    let buffer = '';
    const timer = setTimeout(() => { socket.destroy(); resolve({ timeout: true }); }, timeout);

    socket.on('connect', () => socket.write(`${JSON.stringify({ v: '0.1', id, method, params })}\n`));
    socket.on('data', (chunk) => {
      buffer += chunk.toString();
      for (const line of buffer.split('\n')) {
        try {
          const message = JSON.parse(line);
          if (message.id === id) { clearTimeout(timer); socket.end(); resolve(message); }
        } catch { /* partial */ }
      }
      buffer = buffer.slice(buffer.lastIndexOf('\n') + 1);
    });
    socket.on('error', (error) => { clearTimeout(timer); resolve({ socketError: error.message }); });
  });
}

const show = (label, value) => console.log(`\n=== ${label}\n${JSON.stringify(value, null, 2).slice(0, 6000)}`);

show('host.status', await call('host.status'));
const list = await call('provider.list');
show('provider.list (subscriptions)', (list.result?.providers ?? list.result ?? list).filter?.((p) => ['claude', 'openai', 'gemini'].includes(p.id)).map((p) => ({ id: p.id, status: p.status, detail: p.detail })) ?? list);
show('host.doctor', await call('host.doctor', {}, 90000));

for (const providerId of ['claude', 'openai', 'gemini']) {
  const started = await call('cli.login', { providerId });
  show(`cli.login ${providerId}`, started);
  const loginId = started.result?.loginId;
  if (!loginId) continue;
  let last = null;
  for (let i = 0; i < 20; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    last = await call('cli.login.status', { loginId });
  }
  show(`cli.login.status ${providerId} after 20 s`, last);
  show(`cli.login.cancel ${providerId}`, await call('cli.login.cancel', { loginId }));
}
