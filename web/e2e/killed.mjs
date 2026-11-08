// A runner leaves nothing it started (#558), whether it ends on its own, is interrupted or is
// killed outright: `npm run e2e:killed`. Playwright runs one test (killed.e2e.ts) three times.
// Once it opens a page and ends, and the runner exits as it does after any run; twice it holds
// its browser open and the runner is sent SIGINT, as Ctrl-C sends it, or SIGKILL, as a tool's
// timeout or a killed `cargo xtask ci` would send it. After each, every process carrying the
// run's mark (the glaves its config starts and their helpers, the browser, the test worker)
// must be gone within a guard against a hang; what is left is listed, stopped, and fails the
// check. A runner that ended or was interrupted leaves none of the folders it made either (its
// glaves' homes, its lifeline's); one killed outright can run nothing more to remove them, and
// they go here with the check's own folder.
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const web = join(import.meta.dirname, '..');

/** Waits up to `seconds` for `done`, looking every tenth of a second. */
async function until(seconds, done) {
  for (let looked = 0; looked < seconds * 10 && !done(); looked += 1) {
    await sleep(100);
  }
  return done();
}

/** The signal the runner is sent once the test holds its page, for each way it ends. */
const SIGNALS = { ended: undefined, interrupted: 'SIGINT', killed: 'SIGKILL' };

/**
 * Runs the test as `how` (a key of `SIGNALS`) and says what it left, if anything, as lines of
 * text.
 */
async function run(how) {
  // Under /tmp where there is one: the glaves' sockets are made in it, and a socket's path must
  // stay short.
  const folder = mkdtempSync(join(existsSync('/tmp') ? '/tmp' : tmpdir(), 'glave-killed-'));
  // The mark each process the run starts inherits: unique to this run, so nothing else is
  // ever looked at, let alone stopped.
  const mark = `GLAVE_E2E_KILLED=${folder}`;
  const marked = () =>
    execFileSync('ps', ['axeww', '-o', 'pid=,command='], {
      encoding: 'utf8',
      maxBuffer: 256 * 1024 * 1024,
    })
      .split('\n')
      .filter((line) => line.split(/\s+/u).includes(mark))
      .map((line) => line.trim().split(/\s+/u))
      .map(([pid, command]) => ({ pid: Number(pid), command }))
      .filter(({ pid }) => pid !== process.pid);
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
      env: {
        ...process.env,
        GLAVE_E2E_KILLED: folder,
        GLAVE_E2E_KILLED_HOW: how,
        TMPDIR: folder,
      },
      stdio: ['ignore', 'inherit', 'inherit'],
    },
  );
  const exited = new Promise((resolve) => runner.on('exit', resolve));
  const holding = await until(180, () => existsSync(join(folder, 'holding')));
  const signal = holding ? SIGNALS[how] : 'SIGKILL';
  if (signal !== undefined) {
    runner.kill(signal);
  }
  // A runner that does not end on its own within the guard is killed.
  const timer = setTimeout(() => runner.kill('SIGKILL'), 180_000);
  await exited;
  clearTimeout(timer);
  const ended = await until(30, () => marked().length === 0);
  const left = marked();
  for (const { pid } of left) {
    try {
      process.kill(pid, 'SIGKILL');
    } catch {
      // Gone since it was listed.
    }
  }
  // Looked at once nothing the run started still runs, so none of it could make one again.
  const kept =
    how === 'killed' ? [] : readdirSync(folder).filter((name) => name.startsWith('glave-e2e-'));
  rmSync(folder, { recursive: true, force: true });
  if (!holding) {
    return ['the test never opened its page'];
  }
  return [
    ...(ended ? [] : left.map(({ pid, command }) => `${pid} ${command}`)),
    ...kept.map((name) => join(folder, name)),
  ];
}

for (const how of Object.keys(SIGNALS)) {
  const left = await run(how);
  if (left.length === 0) {
    console.log(`${how}, the runner left nothing behind`);
  } else {
    console.error(
      `${how}, the runner left these, now stopped and removed:\n  ${left.join('\n  ')}`,
    );
    process.exitCode = 1;
  }
}
