import { type BrowserContext, type Page, test } from '@playwright/test';

import { LIVE, type Site, served, signInCode } from './support';

/** How many times each probe tries. */
const TRIES = 100;
/** How long a load may take before it counts as one that never came. */
const HUNG = 10_000;

/** How a load went: it came, the page stayed blank (never committed), or it was only slow. */
type Load = 'ok' | 'blank' | 'slow';

/** What a load that did not come in time left. */
function stuck(page: Page): Load {
  return page.url() === 'about:blank' || page.url() === '' ? 'blank' : 'slow';
}

/** Loads the app in a fresh page of `context`, and waits until it watches the lists. */
async function first(context: BrowserContext, site: Site): Promise<[Load, Page]> {
  const page = await context.newPage();
  const watched = page.waitForRequest(
    (request) => request.headers()['glave-method'] === 'thread/list/watch',
    { timeout: HUNG },
  );
  try {
    await page.goto(`${served(site)}/#code=${await signInCode(site)}`, { timeout: HUNG });
    await watched;
    return ['ok', page];
  } catch {
    // Waited for no longer: what it says once the context closes is no matter.
    watched.catch(() => undefined);
    return [stuck(page), page];
  }
}

/** How the second tab is opened. */
type Way = 'new page' | 'from the first';

/** Opens the second tab `way`, beside `opener`, and waits for its load. */
async function second(opener: Page, site: Site, way: Way): Promise<Load> {
  const url = `${served(site)}/#code=${await signInCode(site)}`;
  let page: Page | null = null;
  try {
    if (way === 'new page') {
      page = await opener.context().newPage();
      await page.goto(url, { timeout: HUNG });
    } else {
      [page] = await Promise.all([
        opener.waitForEvent('popup', { timeout: HUNG }),
        opener.evaluate((address) => {
          window.open(address);
        }, url),
      ]);
      await page.waitForLoadState('load', { timeout: HUNG });
    }
    return 'ok';
  } catch {
    return page === null ? 'slow' : stuck(page);
  }
}

for (const [name, site, way] of [
  ['with COOP, a second tab as a new page', LIVE, 'new page'],
  ['with COOP, a second tab opened from the first', LIVE, 'from the first'],
] as const satisfies readonly (readonly [string, Site, Way])[]) {
  test(`probe stress: ${name}`, async ({ browser }) => {
    test.setTimeout(1_500_000);
    const counts = { first: { blank: 0, slow: 0 }, second: { blank: 0, slow: 0 } };
    for (let i = 0; i < TRIES; i += 1) {
      const context = await browser.newContext();
      const [loaded, opener] = await first(context, site);
      if (loaded !== 'ok') {
        counts.first[loaded] += 1;
      } else {
        const then = await second(opener, site, way);
        if (then !== 'ok') {
          counts.second[then] += 1;
        }
      }
      await context.close();
    }
    console.log(
      `STRESS ${test.info().project.name} | ${name} | tried ${String(TRIES)} | first blank ${String(counts.first.blank)} slow ${String(counts.first.slow)} | second blank ${String(counts.second.blank)} slow ${String(counts.second.slow)}`,
    );
  });
}
