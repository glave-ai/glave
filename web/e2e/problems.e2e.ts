import { EventEmitter } from 'node:events';

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
  await page.goto('/settings');
  await expect(byTestId(page, 'settings-theme')).toBeVisible();
  // Chromium lets both fetches through.
  const left = webkit ? [refusal('/favicon.svg'), SAID] : [SAID];
  expect(problems()).toEqual(left);

  // Left again, however it is left, the same: the session API's refused call is no problem,
  // the icon's and the page's own are.
  await actAsLeft(page, '/settings');
  await page.reload();
  expect([...new Set(problems().slice(left.length))]).toEqual(left);
});

/** The address a stand-in WebKit page is at, and its session API's. */
const HERE = 'http://127.0.0.1:4000/';

/**
 * A WebKit page as `watchForProblems` hears it, its events sent in whatever order a test names:
 * Playwright hands over a page's errors and its navigations on channels of their own, in no
 * promised order.
 */
class Heard extends EventEmitter {
  private address = 'about:blank';
  private readonly frame = {};

  url(): string {
    return this.address;
  }

  mainFrame(): object {
    return this.frame;
  }

  context(): object {
    return { browser: () => ({ browserType: () => ({ name: () => 'webkit' }) }) };
  }

  /** Its main frame asks for `HERE`, as a first load or a leaving does. */
  navigates(): void {
    this.emit('request', {
      isNavigationRequest: () => true,
      frame: () => this.frame,
      url: () => HERE,
    });
  }

  /** The page at `HERE` has loaded. */
  loaded(): void {
    this.address = HERE;
    this.emit('domcontentloaded');
  }

  /** WebKit's error for a call of the session API it refused, split as Playwright hands it. */
  refused(): void {
    const error = new Error('/127.0.0.1:4000/ due to access control checks.');
    error.name = 'Fetch API cannot load http';
    error.stack = `Fetch API cannot load ${HERE} due to access control checks.\n    at post`;
    this.emit('pageerror', error);
  }
}

test('a refusal heard before the page is left, or after the next has loaded, is no problem once it is left', () => {
  const page = new Heard();
  const problems = watchForProblems(page as unknown as Page);
  page.navigates();
  page.loaded();
  page.refused();
  // Nothing has left the page yet, so nothing explains the refusal.
  expect(problems()).toHaveLength(1);
  // The page is left, and the next one loads, before the leaving's refusal is heard (#208).
  page.navigates();
  page.loaded();
  page.refused();
  expect(problems()).toEqual([]);
});
