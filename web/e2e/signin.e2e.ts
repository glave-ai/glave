import { type Request, expect, test } from '@playwright/test';

import {
  ACCOUNTS,
  byTestId,
  expectAccessible,
  heldUntilHeard,
  newTab,
  served,
  signInCode,
  watchForProblems,
} from './support';

/**
 * Signing in by link against a real glave (`specs/one-server.md`, "Finding it"): the link's
 * code signs one browser in, leaves the address bar, and works once; the browser stays signed
 * in after a reload, in a new tab and once every tab is closed, with the session it keeps in its
 * origin's storage, never a cookie; signing out signs every tab out.
 */
test('a link signs one browser in once, and it stays signed in until it signs out', async ({
  page,
  context,
  browser,
}) => {
  await heldUntilHeard(context);
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

  // Nothing of it is a cookie, which every port of this computer's loopback would be sent.
  expect(await context.cookies()).toEqual([]);

  // The code is spent: the same link signs no other browser in.
  const another = await heldUntilHeard(await browser.newContext());
  const elsewhere = await another.newPage();
  await elsewhere.goto(`${glave}/#code=${code}`);
  await expect(byTestId(elsewhere, 'sign-in')).toBeVisible();
  await another.close();

  // A new tab of this browser is signed in, and so is one opened after every tab closed.
  const tab = await newTab(page);
  await tab.goto(`${glave}/settings`);
  await expect(byTestId(tab, 'settings-sign-out')).toBeVisible();
  await page.close();
  await tab.close();
  const reopened = await context.newPage();
  await reopened.goto(`${glave}/`);
  await expect(byTestId(reopened, 'home-composer-input')).toBeVisible();

  // Signing out in one tab signs every tab out.
  const settings = await newTab(reopened);
  await settings.goto(`${glave}/settings`);
  await byTestId(settings, 'settings-sign-out').click();
  await expect(byTestId(settings, 'sign-in')).toBeVisible();
  await expect(byTestId(reopened, 'sign-in')).toBeVisible();
});
