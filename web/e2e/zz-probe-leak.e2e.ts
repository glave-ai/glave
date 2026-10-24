import { type Page, test } from '@playwright/test';

import { LIVE, served, signInCode } from './support';

/** How many pages each probe opens, one after another, in one context. */
const PAGES = 10;

/** Loads the app in `page` and waits until it watches the lists; whether it loaded. */
async function loadWatching(page: Page, url: string): Promise<boolean> {
  const watched = page.waitForRequest(
    (request) => request.headers()['glave-method'] === 'thread/list/watch',
    { timeout: 20_000 },
  );
  try {
    await page.goto(url, { timeout: 20_000 });
    await watched;
    return true;
  } catch {
    return false;
  }
}

/** What `page` saw fail, by method or path. */
function failures(page: Page, into: string[], name: string): void {
  page.on('requestfailed', (request) => {
    const what = request.headers()['glave-method'] ?? new URL(request.url()).pathname;
    into.push(`${name} ${what}: ${request.failure()?.errorText ?? ''}`);
  });
}

for (const how of ['closed', 'reloaded', 'navigated'] as const) {
  test(`probe: pages of one context whose watches are cut by being ${how}`, async ({ browser }) => {
    test.setTimeout(600_000);
    const live = served(LIVE);
    const context = await browser.newContext();
    const failed: string[] = [];
    const outcome: string[] = [];
    let page = await context.newPage();
    failures(page, failed, 'p0');
    let ok = await loadWatching(page, `${live}/#code=${await signInCode(LIVE)}`);
    outcome.push(ok ? 'ok' : 'STALL');
    for (let i = 1; ok && i < PAGES; i += 1) {
      if (how === 'closed') {
        await page.close();
        page = await context.newPage();
        failures(page, failed, `p${String(i)}`);
        ok = await loadWatching(page, `${live}/`);
      } else if (how === 'reloaded') {
        const watched = page.waitForRequest(
          (request) => request.headers()['glave-method'] === 'thread/list/watch',
          { timeout: 20_000 },
        );
        ok = await page
          .reload({ timeout: 20_000 })
          .then(async () => watched)
          .then(
            () => true,
            () => false,
          );
      } else {
        ok = await loadWatching(page, `${live}/${i % 2 === 0 ? '' : 'settings'}`);
      }
      outcome.push(ok ? 'ok' : 'STALL');
    }
    // One more page beside the last, as the tests open a second tab.
    let beside = 'not tried';
    if (ok) {
      const second = await context.newPage();
      beside = (await loadWatching(second, `${live}/`)) ? 'ok' : 'STALL';
    }
    console.log(
      `PROBE ${test.info().project.name} ${how}: ${outcome.join(' ')}; then a second tab: ${beside}\n  failed: ${failed.join(' | ')}`,
    );
    await context.close();
  });
}
