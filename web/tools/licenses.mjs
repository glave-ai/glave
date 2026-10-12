// Checks the licence of every production dependency (what ships in the app) against the
// licences the Rust workspace allows (deny.toml, [licenses] allow), and fails on any other.
// Two additions, each for a kind of package the Rust side has none of: fonts under the SIL
// Open Font License, and 0BSD (tslib), which is more permissive than anything allowed.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const web = new URL('..', import.meta.url).pathname;
const deny = readFileSync(join(web, '../deny.toml'), 'utf8');
const section = deny.slice(deny.indexOf('[licenses]'));
const list = section.slice(
  section.indexOf('allow = ['),
  section.indexOf(']', section.indexOf('allow = [')),
);
const allowed = new Set([...list.matchAll(/"([^"]+)"/g)].map((match) => match[1]));
for (const extra of ['OFL-1.1', '0BSD']) {
  allowed.add(extra);
}

const lock = JSON.parse(readFileSync(join(web, 'package-lock.json'), 'utf8'));
const refused = [];
let checked = 0;
for (const [path, entry] of Object.entries(lock.packages)) {
  if (path === '' || entry.dev === true || entry.devOptional === true) {
    continue;
  }
  checked += 1;
  const manifest = JSON.parse(readFileSync(join(web, path, 'package.json'), 'utf8'));
  const license = String(manifest.license ?? entry.license ?? 'UNKNOWN');
  // An SPDX expression with OR is fine when any one choice is allowed.
  const choices = license.replace(/[()]/g, '').split(/\s+OR\s+/);
  if (!choices.some((choice) => allowed.has(choice.trim()))) {
    refused.push(`${path.replace(/^node_modules\//, '')}: ${license}`);
  }
}

if (refused.length > 0) {
  console.error(`Licences not allowed (deny.toml):\n  ${refused.join('\n  ')}`);
  process.exit(1);
}
console.log(`${checked} production packages, every licence allowed`);
