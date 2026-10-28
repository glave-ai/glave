import { type Page, expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import {
  type Felt,
  expectAnsweredAtOnce,
  expectShownSoon,
  expectStill,
  feltOn,
  forgetFelt,
  watchFeel,
} from './feel';
import { SIGNED_IN, atRest, byTestId } from './support';

// WebKit, as Safari, reports neither layout shifts nor event timings, nor the largest paint.
test.skip(({ browserName }) => browserName !== 'chromium', 'only Chromium reports these entries');

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
