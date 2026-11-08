import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from './support';

/** The folder of a run `killed.mjs` starts; none in any other run. */
const KILLED = process.env['GLAVE_E2E_KILLED'];

/**
 * Whether the run ends on its own (`ended`), or is interrupted or killed outright while this
 * holds on.
 */
const HOW = process.env['GLAVE_E2E_KILLED_HOW'];

// Not a check of the app: only the runs killed.mjs starts run it, and every other run skips
// it. It opens a page, then ends, or holds the browser open until its runner is
// interrupted or killed.
test.skip(KILLED === undefined, 'only in the runs killed.mjs starts');

test('holds a browser open until its runner ends', async ({ page }) => {
  await page.goto('/');
  writeFileSync(join(KILLED ?? '', 'holding'), '');
  if (HOW !== 'ended') {
    // A guard against a hang, never reached: the runner is stopped first.
    await page.waitForTimeout(150_000);
  }
});
