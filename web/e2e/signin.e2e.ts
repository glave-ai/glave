import { type Request, expect, test } from '@playwright/test';

import {
  ACCOUNTS,
  byTestId,
  expectAccessible,
  served,
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
  const glave = served(ACCOUNTS);
  const code = await signInCode(ACCOUNTS);
  // The page's scripts as they load. Its watch of the lists stays open for as long as it lives,
  // so the network is never idle: what the reload must not cut short is a script.
  const loading = new Set<Request>();
  page.on('request', (request) => {
    if (request.resourceType() === 'script') {
      loading.add(request);
    }
  });
  const loaded = (request: Request): void => {
    loading.delete(request);
  };
  page.on('requestfinished', loaded);
  page.on('requestfailed', loaded);
  const followed = page.waitForRequest(
    (request) => request.headers()['glave-method'] === 'thread/list/watch',
  );

  await page.goto(`${glave}/#code=${code}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  expect(page.url()).not.toContain(code);
  // What loads once the page is idle has loaded, so the reload cuts nothing short: the lists
  // followed, the page idle once, and no script still loading.
  await followed;
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        if ('requestIdleCallback' in window) {
          requestIdleCallback(() => {
            resolve();
          });
        } else {
          setTimeout(resolve);
        }
      }),
  );
  await expect.poll(() => loading.size).toBe(0);
  await page.reload();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(problems()).toEqual([]);

  // The code is spent: the same link signs no other tab in.
  const other = await context.newPage();
  await other.goto(`${glave}/#code=${code}`);
  await expect(byTestId(other, 'sign-in')).toBeVisible();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
});
