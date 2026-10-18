import { expect, test } from '@playwright/test';

import {
  ACCOUNTS_HOME,
  ACCOUNTS_URL,
  byTestId,
  expectAccessible,
  signInCode,
  watchForProblems,
} from './support';

/**
 * Signing in by link against a real glave (`specs/one-server.md`, "Finding it"): the link's
 * code signs one tab in, leaves the address bar, and works once; the tab stays signed in after
 * a reload, with the session it keeps.
 */
test('a link signs one tab in once, and it stays signed in after a reload', async ({
  page,
  context,
}) => {
  const problems = watchForProblems(page);
  const code = await signInCode(ACCOUNTS_URL, process.env[ACCOUNTS_HOME] ?? '');

  await page.goto(`${ACCOUNTS_URL}/#code=${code}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  expect(page.url()).not.toContain(code);
  // What loads once the page is idle has loaded, so the reload cuts nothing short.
  await page.waitForLoadState('networkidle');
  await page.reload();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(problems()).toEqual([]);

  // The code is spent: the same link signs no other tab in.
  const other = await context.newPage();
  await other.goto(`${ACCOUNTS_URL}/#code=${code}`);
  await expect(byTestId(other, 'sign-in')).toBeVisible();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
});
