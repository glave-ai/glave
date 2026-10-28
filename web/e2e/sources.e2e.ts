import { type Page } from '@playwright/test';

import { fakeApi } from './fake-api';
import { PAGES, RESEARCH } from './fake-sources';
import {
  SIGNED_IN,
  atRest,
  byTestId,
  expect,
  expectAccessible,
  hold,
  overlaps,
  shownOpacity,
  test,
  watchForProblems,
} from './support';

test('a reply shows the pages it drew on from the web under it, each a link', async ({ page }) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page, { research: true });
  // What the reply drew on comes from the server once the reply shows: held until then.
  const sourcing = await hold(page, '/', 'thread/sources');
  await page.goto(`/t/${RESEARCH}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').last()).toContainText('Tokio 2.0 drops');
  await sourcing.sent;
  const sources = byTestId(page, 'reply-sources');
  await expect(sources).toHaveCount(0);
  sourcing.letGo();
  await expect(byTestId(page, 'reply-sources-sentence')).toHaveText('Sources: 2 pages');
  const links = byTestId(page, 'reply-sources-link');
  await expect(links).toHaveCount(2);
  await expect(links.first()).toHaveAttribute('href', PAGES[0].url);
  await expect(links.first()).toHaveAttribute('target', '_blank');
  await expect(links.first()).toHaveAccessibleName(
    'Announcing Tokio 2.0 tokio.rs/blog/2026-10-tokio-2 (opens in a new tab)',
  );
  // A page that gave no title is named by where it is.
  await expect(links.nth(1)).toHaveAccessibleName(
    'docs.rs/tokio/latest/tokio/ (opens in a new tab)',
  );
  // The search is a step in the server's line, as it read while it ran.
  await byTestId(page, 'thread-run-summary').last().click();
  await expect(byTestId(page, 'thread-receipt').first()).toContainText(
    'Searched the web: Tokio 2.0 release',
  );
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(problems()).toEqual([]);
});

/**
 * Fails if a reply's copy button in the research chat, shown, covers anything else the thread
 * draws: the sources under the reply, a run of calls after a word along the way, another message.
 */
async function expectCopyClear(page: Page): Promise<void> {
  await fakeApi(page, { research: true });
  await page.goto(`/t/${RESEARCH}${SIGNED_IN}`);
  await expect(byTestId(page, 'reply-sources-sentence')).toHaveText('Sources: 2 pages');
  const copies = byTestId(page, 'message-copy');
  await expect(copies).toHaveCount(2);
  await atRest(page);
  const others = [
    byTestId(page, 'reply-sources'),
    byTestId(page, 'thread-run'),
    byTestId(page, 'thread-steps'),
    byTestId(page, 'thread-message'),
  ];
  for (const copy of await copies.all()) {
    // Shown as a pointer shows it, on its message; a touch screen always shows it.
    await copy.locator('xpath=ancestor::*[@data-testid="thread-message"]').hover();
    await expect.poll(() => shownOpacity(copy)).toBe(1);
    const button = await copy.boundingBox();
    for (const other of others) {
      for (const each of await other.all()) {
        const owner = await each.evaluate(
          (element, held) => element.contains(held),
          await copy.elementHandle(),
        );
        if (!owner) {
          expect(
            overlaps(button, await each.boundingBox()),
            `the copy button ${JSON.stringify(button)} over ${String(await each.getAttribute('data-testid'))} ${JSON.stringify(await each.boundingBox())}`,
          ).toBe(false);
        }
      }
    }
  }
}

for (const width of [1440, 900] as const) {
  test(`at ${String(width)} px, a reply's copy button covers nothing that follows it`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await expectCopyClear(page);
  });
}

test.describe('on a touch screen', () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test("a reply's copy button, always shown, covers nothing that follows it", async ({ page }) => {
    await expectCopyClear(page);
  });
});
