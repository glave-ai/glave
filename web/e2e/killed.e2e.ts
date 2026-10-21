import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { test } from '@playwright/test';

/** The folder of the run `killed.mjs` kills outright; none in any other run. */
const KILLED = process.env['GLAVE_E2E_KILLED'];

// Not a check of the app: in the run killed.mjs kills outright, it holds a browser open until
// then. Every other run skips it.
test('holds a browser open until its runner is killed', async ({ page }) => {
  test.skip(KILLED === undefined, 'only in the run killed.mjs kills');
  await page.goto('/');
  writeFileSync(join(KILLED ?? '', 'holding'), '');
  // A guard against a hang, never reached: the runner is killed first.
  await page.waitForTimeout(150_000);
});
