// A runner killed outright leaves nothing it started (#558): `npm run e2e:killed`. Playwright
// runs one test that holds a browser open; once it does, the runner is sent SIGKILL, as a
// tool's timeout or a killed `cargo xtask ci` would send it, and every process carrying this
// run's mark (the glaves its config starts and their helpers, the browser, the test worker)
// must end within a guard against a hang. Whatever is left is listed, stopped, and fails this.
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const web = join(import.meta.dirname, '..');
// Under /tmp where there is one: the glaves' sockets are made in it, and a socket's path must
// stay short.
const folder = mkdtempSync(join(existsSync('/tmp') ? '/tmp' : tmpdir(), 'glave-killed-'));
// The mark each process the run starts inherits: unique to this run, so nothing else is ever
// looked at, let alone stopped.
const mark = `GLAVE_E2E_KILLED=${folder}`;

/** The processes still carrying this run's mark, by id, with what they run. */
function marked() {
  const listed = execFileSync('ps', ['axeww', '-o', 'pid=,command='], { encoding: 'utf8' });
  return listed
    .split('\n')
    .filter((line) => line.split(/\s+/u).includes(mark))
    .map((line) => line.trim().split(/\s+/u))
    .map(([pid, command]) => ({ pid: Number(pid), command }))
    .filter(({ pid }) => pid !== process.pid);
}

/** Waits up to `seconds` for `done`, looking every tenth of a second. */
async function until(seconds, done) {
  for (let looked = 0; looked < seconds * 10 && !done(); looked += 1) {
    await sleep(100);
  }
  return done();
}

const runner = spawn(
  process.execPath,
  [
    join(web, 'node_modules/@playwright/test/cli.js'),
    'test',
    '-c',
    'e2e/playwright.config.ts',
    'killed.e2e.ts',
    '--project',
    'chromium',
    `--output=${join(folder, 'results')}`,
  ],
  {
    cwd: web,
    env: { ...process.env, GLAVE_E2E_KILLED: folder, TMPDIR: folder },
    stdio: ['ignore', 'inherit', 'inherit'],
  },
);
const holding = await until(180, () => existsSync(join(folder, 'holding')));
const started = marked().length;
runner.kill('SIGKILL');
const ended = await until(30, () => marked().length === 0);
const left = marked();
for (const { pid } of left) {
  process.kill(pid, 'SIGKILL');
}
rmSync(folder, { recursive: true, force: true });
if (!holding) {
  console.error('the test never held its browser open');
  process.exitCode = 1;
} else if (!ended) {
  const listed = left.map(({ pid, command }) => `  ${pid} ${command}`).join('\n');
  console.error(
    `a runner killed outright left ${left.length} of the ${started} processes it started, now stopped:\n${listed}`,
  );
  process.exitCode = 1;
} else {
  console.log(`a runner killed outright left none of the ${started} processes it started`);
}
