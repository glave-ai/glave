import { expect, test } from '@playwright/test';

import { fakeApi } from './fake-api';
import { PAGES, RESEARCH } from './fake-sources';
import { SIGNED_IN, byTestId, expectAccessible, hold, watchForProblems } from './support';

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
  // The page that failed to load is none.
  await expect(sources).not.toContainText('missing');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(problems()).toEqual([]);
});
