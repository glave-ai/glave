import { type Page } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { discovered } from './fake-signin';
import { SIGNED_IN, builtChunk, byTestId, expect, expectAccessible, test } from './support';

/**
 * Answers the app's file that defines the component `selector` with a 404 from now on, as when
 * glave was updated while the tab was open and that file's name is gone; the name it had, so a
 * test can let it load again.
 */
async function gone(page: Page, selector: string): Promise<string> {
  const found = new RegExp(`selectors:\\[\\[["'\`]${selector}["'\`]\\]\\]`, 'u');
  const chunk = `**/${builtChunk((code) => found.test(code))}`;
  await page.route(chunk, (route) => route.fulfill({ status: 404 }));
  return chunk;
}

test('a page whose code cannot load says so in its place, and Reload opens it', async ({
  page,
}) => {
  await fakeApi(page);
  const chunk = await gone(page, 'app-thread');
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  // The page that says so opens in the thread's place, at the thread's own address.
  await expect(byTestId(page, 'unloaded-sentence')).toHaveText(
    'This part of glave could not load.',
  );
  await expect(page).toHaveURL(new RegExp(`/t/${THREAD}$`, 'u'));
  await expectAccessible(page);
  // The network is back: Reload opens the thread.
  await page.unroute(chunk);
  await byTestId(page, 'unloaded-reload').click();
  await expect(byTestId(page, 'thread-title')).toHaveText('Make the tests pass');
});

test('a part that cannot load once glave was updated says to reload to continue', async ({
  page,
}) => {
  await fakeApi(page);
  await gone(page, 'app-new-project');
  // glave was updated after the page loaded: the server says a version after the page's.
  let version = discovered().version;
  await page.route('**/*', async (route) => {
    const request = route.request();
    if (request.headers()['glave-method'] !== 'server/discover') {
      await route.fallback();
      return;
    }
    const { id } = request.postDataJSON() as { readonly id: number };
    await route.fulfill({ json: { jsonrpc: '2.0', id, result: discovered(version) } });
  });
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  version = '0.0.1';
  // New project's code is gone with the old version: it says so where it would open.
  await byTestId(page, 'nav-new-project').click();
  await expect(byTestId(page, 'unloaded-sentence')).toHaveText(
    'glave was updated. Reload to continue.',
  );
  await expect(byTestId(page, 'unloaded-reload')).toBeVisible();
  await expectAccessible(page);
});
