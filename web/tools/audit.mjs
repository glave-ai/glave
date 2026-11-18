// Asks the npm registry's bulk advisory endpoint about every production dependency (what ships
// in the app: each package in package-lock.json but the dev ones, the set `npm audit
// --omit=dev` asks about) and fails on any high or critical advisory. The endpoint answers only
// with the advisories that cover the versions it is asked about. This script asks it directly
// rather than through `npm audit`, since npm 10, which Node 22 bundles, falls back to the
// quick-audit endpoint the registry is retiring whenever the bulk one fails (#1557). An answer
// that is no verdict (an error, no answer in time, or a body that isn't advisories) fails too,
// and says so apart from advisories found.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const web = new URL('..', import.meta.url).pathname;
// The registry npm is set to: `npm run` passes it on when it isn't npm's default.
const registry = process.env.npm_config_registry ?? 'https://registry.npmjs.org/';
const endpoint = new URL('-/npm/v1/security/advisories/bulk', registry.replace(/\/?$/, '/'));
const seconds = 30;

const lock = JSON.parse(readFileSync(join(web, 'package-lock.json'), 'utf8'));
const versions = {};
for (const [path, entry] of Object.entries(lock.packages)) {
  if (path === '' || entry.dev === true || entry.link === true || entry.version === undefined) {
    continue;
  }
  // An alias (`npm:`) is asked about by the package it stands for.
  const name = entry.name ?? path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length);
  versions[name] = [...new Set([...(versions[name] ?? []), entry.version])];
}

const noVerdict = (why) => {
  console.error(
    `The npm registry's advisory service didn't answer (${why}), so there is no verdict on ` +
      'the production dependencies, good or bad: run the audit again.',
  );
  process.exit(1);
};

let advisories;
try {
  const response = await fetch(endpoint, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(versions),
    signal: AbortSignal.timeout(seconds * 1000),
  });
  const body = await response.text();
  if (!response.ok) {
    noVerdict(`${response.status} ${response.statusText} from ${endpoint}: ${body.slice(0, 200)}`);
  }
  advisories = JSON.parse(body);
} catch (error) {
  noVerdict(
    error.name === 'TimeoutError'
      ? `nothing from ${endpoint} in ${seconds} seconds`
      : `${error.cause?.message ?? error.message} from ${endpoint}`,
  );
}
// Advisories are a list for each package, each advisory with its severity.
const isAdvisories = (answer) =>
  answer?.constructor === Object &&
  Object.values(answer).every((list) => Array.isArray(list) && list.every((one) => one?.severity));
if (!isAdvisories(advisories)) {
  noVerdict(`an answer from ${endpoint} that isn't advisories`);
}

const serious = Object.entries(advisories).flatMap(([name, list]) =>
  list
    .filter(({ severity }) => severity === 'high' || severity === 'critical')
    .map(({ severity, title, url }) => `${name} ${versions[name]}: ${title} (${severity}, ${url})`),
);
if (serious.length > 0) {
  console.error(
    `High or critical advisories in production dependencies:\n  ${serious.join('\n  ')}`,
  );
  process.exit(1);
}
console.log(`${Object.keys(versions).length} production packages, no high or critical advisory`);
