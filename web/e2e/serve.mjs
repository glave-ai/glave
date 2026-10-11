// Serves the built app (dist/app/browser) as `glave serve --http` does, for the end-to-end
// tests: the same Content Security Policy, a fresh nonce in index.html on every load, and
// index.html for any path that is not a file (the app's routes). Node's standard library only.
import { randomBytes } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { extname, join, normalize, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../dist/app/browser/', import.meta.url));
const port = Number(process.env['PORT'] ?? 4310);

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.woff2': 'font/woff2',
  '.woff': 'font/woff',
  '.json': 'application/json',
};

/** The policy glave serves the app with (crates/session/src/http/site.rs); keep them the same. */
function policy(nonce) {
  return [
    "default-src 'none'",
    "script-src 'self'",
    `style-src 'self' 'nonce-${nonce}'`,
    "font-src 'self'",
    "img-src 'self'",
    "connect-src 'self'",
    "manifest-src 'self'",
    "base-uri 'self'",
    "form-action 'none'",
    "frame-ancestors 'none'",
  ].join('; ');
}

async function answer(request, response) {
  const path = decodeURIComponent(new URL(request.url ?? '/', 'http://local').pathname);
  const file = normalize(join(root, path));
  const nonce = randomBytes(16).toString('base64');
  response.setHeader('Content-Security-Policy', policy(nonce));
  response.setHeader('X-Content-Type-Options', 'nosniff');
  response.setHeader('Referrer-Policy', 'no-referrer');
  if (file.startsWith(root) && !file.endsWith(sep) && extname(file) !== '') {
    try {
      const body = await readFile(file);
      response.writeHead(200, {
        'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream',
      });
      response.end(body);
      return;
    } catch {
      response.writeHead(404).end();
      return;
    }
  }
  const page = (await readFile(join(root, 'index.html'), 'utf8')).replaceAll(
    'GLAVE_CSP_NONCE',
    nonce,
  );
  response.writeHead(200, { 'Content-Type': TYPES['.html'], 'Cache-Control': 'no-store' });
  response.end(page);
}

createServer((request, response) => {
  answer(request, response).catch(() => response.writeHead(500).end());
}).listen(port, '127.0.0.1');
