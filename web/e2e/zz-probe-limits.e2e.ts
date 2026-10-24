import { type Page, expect, test } from '@playwright/test';

import { LIVE, byTestId, served, signInCode } from './support';

/** Opens `count` list watches from `page`, each holding its own connection, and leaves them. */
async function holdWatches(page: Page, count: number): Promise<void> {
  await page.evaluate((count) => {
    const kept = JSON.parse(localStorage.getItem('glave-session') ?? '{}') as { token?: string };
    for (let i = 0; i < count; i += 1) {
      void fetch('/', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'text/event-stream, application/json',
          Authorization: `Bearer ${kept.token ?? ''}`,
          'Glave-Method': 'thread/list/watch',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1000 + i,
          method: 'thread/list/watch',
          params: { after: new Date().toISOString() },
        }),
      }).then(async (response) => {
        // Read on, so the stream stays open.
        const reader = response.body?.getReader();
        for (;;) {
          const next = await reader?.read();
          if (next === undefined || next.done) {
            return;
          }
        }
      });
    }
  }, count);
}

for (const held of [4, 5, 6]) {
  test(`probe: a second tab of the same browser loads while the first holds ${String(held)} watches`, async ({
    browser,
  }) => {
    const context = await browser.newContext();
    const [first, second] = [await context.newPage(), await context.newPage()];
    const live = served(LIVE);
    await first.goto(`${live}/#code=${await signInCode(LIVE)}`);
    await expect(byTestId(first, 'home-composer-input')).toBeVisible();
    // The app's own watch is one; the rest are added.
    await holdWatches(first, held - 1);
    const began = Date.now();
    let loaded = 'loaded';
    try {
      await second.goto(`${live}/`, { timeout: 15_000 });
    } catch {
      loaded = 'NOT loaded';
    }
    console.log(
      `${test.info().project.name}: first tab holds ${String(held)} watches: second tab ${loaded} in ${String(Date.now() - began)} ms`,
    );
    await context.close();
  });
}

test('probe: a long-lived fetch started before load delays the load event', async ({ browser }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  const live = served(LIVE);
  await page.goto(`${live}/#code=${await signInCode(LIVE)}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  // From now on, each document of this page opens a watch before anything else of it runs.
  await page.addInitScript(() => {
    const kept = JSON.parse(localStorage.getItem('glave-session') ?? '{}') as { token?: string };
    void fetch('/', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Accept: 'text/event-stream, application/json',
        Authorization: `Bearer ${kept.token ?? ''}`,
        'Glave-Method': 'thread/list/watch',
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 999,
        method: 'thread/list/watch',
        params: { after: new Date().toISOString() },
      }),
    });
  });
  const began = Date.now();
  let loaded = 'fired';
  try {
    await page.goto(`${live}/`, { timeout: 15_000 });
  } catch {
    loaded = 'NEVER fired';
  }
  console.log(
    `${test.info().project.name}: a fetch held from before load: load ${loaded} in ${String(Date.now() - began)} ms`,
  );
  await context.close();
});
