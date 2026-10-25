import { expect, test } from '@playwright/test';

import { fakeApi } from './fake-api';
import { DOCS_COMMAND, DOCS_SERVER, SERVING } from './fake-commands';
import { SIGNED_IN, byTestId, expectAccessible, hold, watchForProblems } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the tools show what a thread runs in the background, and Stop ends it (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    const api = await fakeApi(page, { commands: true });
    await page.goto(`/t/${SERVING}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-message').last()).toContainText('The docs site is running');
    await byTestId(page, 'tools-handle').press('Enter');
    const tab = byTestId(page, 'tools-tabs-tab').nth(2);
    // The thread recorded the command starting, so the tab counts it before it opens.
    await expect(tab).toHaveText('Commands (1)');
    await tab.click();
    const text = byTestId(page, 'command-text');
    await expect(text).toHaveText(DOCS_SERVER);
    await expect(byTestId(page, 'command')).toContainText('Still running');
    await expect(byTestId(page, 'command-time')).toHaveText(/^Running for 2m \d\ds$/u);
    // One line, cut short in the panel; focus shows all of it.
    expect(await text.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
    await text.focus();
    await expect(page.getByRole('tooltip').filter({ hasText: DOCS_SERVER })).toBeVisible();
    await expectAccessible(page);

    const stop = byTestId(page, 'command-stop');
    await expect(stop).toHaveAccessibleName(`Stop ${DOCS_SERVER}`);
    // It stays stopping until the test has seen it.
    const stopping = await hold(page, '/', 'command/stop');
    await stop.click();
    await expect(stop).toHaveText('Stopping…');
    stopping.letGo();
    await expect(byTestId(page, 'command')).toHaveCount(0);
    await expect(byTestId(page, 'commands-empty')).toContainText(
      'Nothing running in the background',
    );
    await expect(tab).toHaveText('Commands');
    const stops = api.requests.filter((request) => request.method === 'command/stop');
    expect(stops.map((request) => request.params)).toEqual([
      { thread: SERVING, command: DOCS_COMMAND },
    ]);
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}
