import { type Page } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import {
  type Felt,
  FEEL,
  expectAnsweredAtOnce,
  expectShownSoon,
  expectStill,
  feltOn,
  forgetFelt,
  watchFeel,
} from './feel';
import { SIGNED_IN, atRest, byTestId, expect, test } from './support';

// Run in Chromium alone, one at a time once the rest of the run is done (playwright.config.ts,
// the `feel` project): WebKit, as Safari, reports neither layout shifts nor event timings, nor
// the largest paint, and other browsers drawing beside these would stretch their times.

/** The screens the app opens on, each with what shows once it has loaded. */
const OPENED = [
  { name: 'the Chats', path: '/', shown: 'nav-thread' },
  { name: 'a thread', path: `/t/${THREAD}`, shown: 'thread-message' },
] as const;

/** What the person feels as `page` opens at `path` and comes to rest, `shown` on it. */
async function opening(page: Page, path: string, shown: string): Promise<Felt> {
  await watchFeel(page);
  await fakeApi(page);
  await page.goto(`${path}${SIGNED_IN}`);
  await expect(byTestId(page, shown).last()).toBeVisible();
  await expect(byTestId(page, 'nav-thread').first()).toBeVisible();
  await atRest(page);
  return feltOn(page);
}

for (const opened of OPENED) {
  test(`opening ${opened.name} shows its content soon`, async ({ page }) => {
    expectShownSoon(await opening(page, opened.path, opened.shown));
  });

  test(`nothing moves on opening ${opened.name}`, async ({ page }) => {
    expectStill(await opening(page, opened.path, opened.shown));
  });
}

test('going from thread to thread in the side nav answers each click at once, and nothing moves', async ({
  page,
}) => {
  await opening(page, `/t/${THREAD}`, 'thread-message');
  await forgetFelt(page);
  const threads = byTestId(page, 'nav-thread');
  const count = await threads.count();
  for (let each = 0; each < count; each += 1) {
    const thread = threads.nth(each);
    await thread.click();
    await expect(thread).toHaveAttribute('aria-current', 'page');
    await expect(byTestId(page, 'thread-composer')).toBeVisible();
    await atRest(page);
  }
  const felt = await feltOn(page);
  expectAnsweredAtOnce(felt);
  expectStill(felt);
});

test('opening Settings from a thread shows it at once over a slow network, its code already fetched', async ({
  page,
}) => {
  // The app fetches Settings' code on its own once the thread has opened: the script that
  // draws it comes before any click.
  const fetched = page.waitForResponse(
    async (response) =>
      response.url().endsWith('.js') && (await response.text()).includes('settings-theme'),
  );
  await opening(page, `/t/${THREAD}`, 'thread-message');
  const settings = (await fetched).url();
  // Settings' code and all it imports have run: importing it again only reads it.
  await page.evaluate(async (url) => {
    await import(url);
  }, settings);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Network.enable');
  await cdp.send('Network.emulateNetworkConditions', {
    offline: false,
    latency: FEEL.slowNetwork,
    downloadThroughput: -1,
    uploadThroughput: -1,
  });
  const scripts: string[] = [];
  page.on('request', (request) => {
    if (request.resourceType() === 'script') {
      scripts.push(request.url());
    }
  });
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'nav-settings')).toBeVisible();
  const shown = await page.evaluate(
    async () =>
      new Promise<number>((resolve) => {
        const start = performance.now();
        const look = (): void => {
          if (document.querySelector('[data-testid="settings-theme"]') === null) {
            requestAnimationFrame(look);
          } else {
            resolve(performance.now() - start);
          }
        };
        document.querySelector<HTMLElement>('[data-testid="nav-settings"]')?.click();
        requestAnimationFrame(look);
      }),
  );
  await expect(byTestId(page, 'settings-theme')).toBeVisible();
  expect(scripts, 'scripts fetched on opening Settings').toEqual([]);
  expect(shown, 'from the click until Settings shows').toBeLessThanOrEqual(FEEL.pageShown);
});
