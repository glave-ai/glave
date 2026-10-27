import { expect, test } from '@playwright/test';

import { fakeApi } from './fake-api';
import { DOCS_COMMAND, DOCS_SERVER, DOCS_STARTED, SERVING } from './fake-commands';
import { SIGNED_IN, byTestId, expectAccessible, hold, watchForProblems } from './support';

/** The page's clock, held two minutes and five seconds after the docs site started. */
const NOW = new Date(Date.parse(DOCS_STARTED) + 125_000);

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the panel shows what a chat runs in the background, and Stop ends it (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.clock.setFixedTime(NOW);
    const api = await fakeApi(page, { commands: true });
    await page.goto(`/t/${SERVING}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-message').last()).toContainText('The docs site is running');
    const running = byTestId(page, 'panel-running');
    await expect(running).toContainText('Running');
    // One line: what it runs, cut short, and how long it has run; it opens to the whole
    // command and the server's sentence about it.
    const text = byTestId(page, 'command-text');
    await expect(text).toHaveText(DOCS_SERVER);
    await expect(byTestId(page, 'command-time')).toHaveText('2m 05s');
    expect(await text.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
    const summary = byTestId(page, 'command-summary');
    await expect(summary).toBeHidden();
    await byTestId(page, 'command-more').locator('summary').click();
    await expect(summary).toContainText('Still running');
    await expect(byTestId(page, 'command-more')).toContainText(DOCS_SERVER);
    await expectAccessible(page);

    const stop = byTestId(page, 'command-stop');
    await expect(stop).toHaveAccessibleName(`Stop ${DOCS_SERVER}`);
    // It stays stopping until the test has seen it.
    const stopping = await hold(page, '/', 'command/stop');
    await stop.focus();
    await page.keyboard.press('Enter');
    await expect(stop).toHaveAccessibleName(`Stopping ${DOCS_SERVER}…`);
    await expect(stop).toBeDisabled();
    stopping.letGo();
    // Nothing runs now, and a chat has nothing else for its panel: it goes, and the keyboard's
    // place goes to the composer.
    await expect(byTestId(page, 'command')).toHaveCount(0);
    await expect(running).toHaveCount(0);
    await expect(byTestId(page, 'panel-panel')).toBeHidden();
    await expect(byTestId(page, 'thread-panel')).toHaveCount(0);
    await expect(byTestId(page, 'thread-composer-input')).toBeFocused();
    const stops = api.requests.filter((request) => request.method === 'command/stop');
    expect(stops.map((request) => request.params)).toEqual([
      { thread: SERVING, command: DOCS_COMMAND },
    ]);
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('on a phone, a sheet left with nothing closes, and does not open again by itself', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.clock.setFixedTime(NOW);
  await fakeApi(page, { commands: true });
  await page.goto(`/t/${SERVING}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').last()).toContainText('The docs site is running');
  const panel = byTestId(page, 'panel-panel');
  const button = byTestId(page, 'thread-panel');
  await expect(panel).toBeHidden();
  await expect(button).toHaveAttribute('aria-expanded', 'false');
  await button.click();
  await expect(panel).toHaveAttribute('aria-modal', 'true');
  await expect(byTestId(page, 'panel-heading')).toHaveText('This thread');
  await byTestId(page, 'command-stop').click();
  await expect(byTestId(page, 'command')).toHaveCount(0);
  await expect(panel).toBeHidden();
  await expect(panel).not.toHaveAttribute('aria-modal', 'true');
  await expect(byTestId(page, 'thread-composer-input')).toBeVisible();
  await expectAccessible(page);
});
