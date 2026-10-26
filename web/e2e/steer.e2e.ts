import { expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { letGo } from './fake-gate';
import { steerTurn } from './fake-steer';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a message sent while the agent works waits, then is read where the model saw it (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await steerTurn(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
    const composer = byTestId(page, 'thread-composer-input');
    await composer.fill('Set the port to 3000');
    await composer.press('Enter');
    const stop = byTestId(page, 'thread-composer-stop');
    await expect(stop).toBeVisible();
    // The composer stays usable while the agent works; Send comes back beside Stop.
    await expect(composer).toBeEnabled();
    await composer.fill('Actually, use 4000');
    await byTestId(page, 'thread-composer-send').click();
    const waiting = byTestId(page, 'thread-queued');
    await expect(waiting).toContainText('Actually, use 4000');
    await expect(waiting).toContainText('Waiting to be read');
    await expect(stop).toBeVisible();
    await expectAccessible(page);
    // The model reads it at its next step: it moves into the conversation, read.
    await letGo(page, 'read');
    await expect(waiting).toHaveCount(0);
    const read = byTestId(page, 'thread-message-read');
    await expect(read).toHaveText('Read while working');
    await expect(byTestId(page, 'thread-message').last()).toContainText('Using 4000 instead.');
    await expect(stop).toHaveCount(0);
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}
