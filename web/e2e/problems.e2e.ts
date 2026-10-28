import { type Page } from '@playwright/test';

import { fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, test, watchForProblems } from './support';

/** What the page says went wrong as it is left, after the fetches it starts then. */
const SAID = 'something went wrong as the page was left';

/**
 * Has the page, once it is being left (its loads stopped, after beforeunload), call the session
 * API, fetch its icon and say something went wrong; the next page, at `next`, waits until it
 * has.
 */
async function actAsLeft(page: Page, next: string): Promise<void> {
  await page.evaluate((said) => {
    addEventListener('beforeunload', () => {
      setTimeout(() => {
        const discover = { jsonrpc: '2.0', id: 0, method: 'server/discover', params: {} };
        void fetch('/', { method: 'POST', body: JSON.stringify(discover) }).catch(() => undefined);
        void fetch('/favicon.svg').catch(() => undefined);
        // A page saying something went wrong is what this tests.
        // eslint-disable-next-line no-console -- see above
        console.error(said);
      });
    });
  }, SAID);
  const said = page.waitForEvent('console', (message) => message.text() === SAID);
  await page.route(
    `**${next}`,
    async (route) => {
      await said;
      await route.fallback();
    },
    { times: 1 },
  );
}

test('as a page is left, WebKit refusing a call of the session API is no problem; all else said is', async ({
  page,
  browserName,
}) => {
  const problems = watchForProblems(page);
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  const refusal = (path: string): string =>
    `Fetch API cannot load ${new URL(path, page.url()).href} due to access control checks.`;
  const webkit = browserName === 'webkit';

  await actAsLeft(page, '/settings');
  await problems.leave('/settings');
  await expect(byTestId(page, 'settings-theme')).toBeVisible();
  // Chromium lets both fetches through.
  const left = webkit ? [refusal('/favicon.svg'), SAID] : [SAID];
  expect(problems()).toEqual(left);

  // Left without saying so, the session API's refused call counts too, as does the app's own if
  // one began as the page went, in the same words: each line once.
  await actAsLeft(page, '/tasks');
  await page.goto('/tasks');
  expect([...new Set(problems().slice(left.length))]).toEqual(
    webkit ? [refusal('/'), refusal('/favicon.svg'), SAID] : [SAID],
  );
});
