// Dev-only: serves the *built* frontend (`app/dist`) so a headless browser can be pointed at exactly
// the files the installer embeds. If the UI renders here and not in the window, the problem is the
// window's environment (WebView2, asset protocol); if it does not render here either, it is the bundle.
import { createServer } from 'node:http';
import { readFileSync, existsSync, statSync } from 'node:fs';
import { extname, join, normalize } from 'node:path';

const root = join(import.meta.dirname, '..', 'sdc', 'app', 'dist');
const port = Number(process.argv[2] ?? 4599);

const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
};

if (!existsSync(root)) {
  console.error(`no build at ${root} — run pnpm build first`);
  process.exit(2);
}

createServer((request, response) => {
  const path = normalize(decodeURIComponent((request.url ?? '/').split('?')[0]));
  let file = join(root, path === '/' ? 'index.html' : path);

  if (!file.startsWith(root) || !existsSync(file) || statSync(file).isDirectory()) {
    file = join(root, 'index.html');
  }

  response.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' });
  response.end(readFileSync(file));
}).listen(port, '127.0.0.1', () => {
  console.log(`serving ${root} on http://127.0.0.1:${port}`);
});
