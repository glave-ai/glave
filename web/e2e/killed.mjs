// A runner leaves nothing it started (#558), whether it ends on its own or is killed outright:
// `npm run e2e:killed`. Playwright runs one test (killed.e2e.ts) twice. Once it opens a page and
// ends, and the runner exits as it does after any run; once it holds its browser open and the
// runner is sent SIGKILL, as a tool's timeout or a killed `cargo xtask ci` would send it. After
// each, every process carrying the run's mark (the glaves its config starts and their helpers,
// the browser, the test worker) must be gone within a guard against a hang; what is left is
// listed, stopped, and fails the check.
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
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

/**
 * Runs the test as `how` ('ended' or 'killed') and says what it left running, if anything, as
 * lines of text.
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
  if (how === 'killed' || !holding) {
    runner.kill('SIGKILL');
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
  rmSync(folder, { recursive: true, force: true });
  if (!holding) {
    return ['the test never opened its page'];
  }
  return ended ? [] : left.map(({ pid, command }) => `${pid} ${command}`);
}

for (const how of ['ended', 'killed']) {
  const left = await run(how);
  if (left.length === 0) {
    console.log(`a runner that ${how} left nothing it started`);
  } else {
    console.error(`a runner that ${how} left, now stopped:\n  ${left.join('\n  ')}`);
    process.exitCode = 1;
  }
}
