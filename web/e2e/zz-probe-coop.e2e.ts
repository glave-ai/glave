import { expect, test } from '@playwright/test';

import { HERE_WEBKIT, LIVE, type Site, byTestId, served, signInCode } from './support';

/** How many times each probe opens a second tab beside a first that has just loaded. */
const TRIES = 40;

for (const [name, site] of [
  ['with COOP', LIVE],
  ['without COOP', HERE_WEBKIT],
] as const satisfies readonly (readonly [string, Site])[]) {
  test(`probe: a second tab loads beside a first that just loaded, ${name}`, async ({
    browser,
  }) => {
    test.setTimeout(900_000);
    const glave = served(site);
    const stalls: string[] = [];
    let first = 0;
    for (let i = 0; i < TRIES; i += 1) {
      const context = await browser.newContext();
      const [watching, working] = [await context.newPage(), await context.newPage()];
      const watched = new Promise<void>((resolve) => {
        watching.on('request', (request) => {
          if (request.headers()['glave-method'] === 'thread/list/watch') {
            resolve();
          }
        });
      });
      const began = Date.now();
      try {
        await watching.goto(`${glave}/#code=${await signInCode(site)}`, { timeout: 20_000 });
      } catch {
        first += 1;
        await context.close();
        continue;
      }
      await watched;
      try {
        await working.goto(`${glave}/#code=${await signInCode(site)}`, { timeout: 20_000 });
        await expect(byTestId(working, 'home-composer-input')).toBeVisible();
      } catch {
        const at = await working
          .evaluate(() => `${location.href} ${document.readyState}`)
          .catch((error: unknown) => String(error));
        stalls.push(`try ${String(i)} after ${String(Date.now() - began)} ms: B at ${at}`);
      }
      await context.close();
    }
    console.log(
      `PROBE ${name}: ${String(stalls.length)} of ${String(TRIES)} second tabs never loaded; ${String(first)} first tabs never loaded\n${stalls.join('\n')}`,
    );
  });
}
