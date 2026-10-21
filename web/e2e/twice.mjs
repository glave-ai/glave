// The end-to-end tests run twice at once on this machine, as two `cargo xtask ci` side by side
// run them: `npm run e2e:twice [-- <Playwright's arguments>]`. Each run's glaves listen on ports
// the system picks (playwright.config.ts), so neither meets the other's, and this fails unless
// both pass. Each run keeps its results in a folder of its own, and marks its lines with its
// number.
import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

const web = join(import.meta.dirname, '..');

/** Runs the tests as run `name`, and says whether they passed. */
function run(name) {
  const child = spawn(
    'playwright',
    [
      'test',
      '-c',
      'e2e/playwright.config.ts',
      `--output=test-results/run-${name}`,
      ...process.argv.slice(2),
    ],
    { cwd: web, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  for (const output of [child.stdout, child.stderr]) {
    createInterface({ input: output }).on('line', (line) => {
      console.log(`[run ${name}] ${line}`);
    });
  }
  return new Promise((resolve, reject) => {
    child.on('error', reject);
    child.on('close', (code) => resolve(code === 0));
  });
}

const passed = await Promise.all(['1', '2'].map(run));
console.log(`run 1 ${passed[0] ? 'passed' : 'failed'}, run 2 ${passed[1] ? 'passed' : 'failed'}`);
if (!passed.every(Boolean)) {
  process.exitCode = 1;
}
