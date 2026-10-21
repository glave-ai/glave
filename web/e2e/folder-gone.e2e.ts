import { type Page, expect, test } from '@playwright/test';

import { fakeApi } from './fake-api';
import { GONE_MESSAGE, LOST, PLAYGROUND, WAS_AT } from './fake-gone';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

/** Fails if the page shows a path the person never typed, an error code, or the OS's words. */
async function expectPlainWords(page: Page): Promise<void> {
  const text = await page.locator('body').innerText();
  for (const raw of [WAS_AT, '/private', '-32029', 'os error', 'No such file', 'went wrong']) {
    expect(text, `the page shows ${raw}`).not.toContain(raw);
  }
}

/** Opens the thread of playground, the project whose folder is gone, as it was left. */
async function openLost(page: Page): Promise<void> {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`/t/${LOST}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').last()).toContainText('Here is a first sketch.');
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a project whose folder is gone says so plainly, and Find it lets the next turn run (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    const api = await fakeApi(page, { gone: true });
    await openLost(page);

    // In place of the composer, and on the project in the side nav: the server's words.
    await expect(byTestId(page, 'thread-gone-message')).toHaveText(GONE_MESSAGE);
    await expect(byTestId(page, 'thread-gone-find')).toHaveText('Find it');
    await expect(byTestId(page, 'thread-gone-remove')).toHaveText('Remove the project');
    await expect(byTestId(page, 'thread-composer')).toBeHidden();
    await expect(byTestId(page, 'nav-gone-message')).toHaveText(GONE_MESSAGE);
    await expect(byTestId(page, 'nav-gone-find')).toBeVisible();
    await expect(byTestId(page, 'nav-gone-remove')).toBeVisible();

    // The tools say nothing changed and nothing runs, rather than an error.
    await byTestId(page, 'tools-handle').press('Enter');
    const tabs = byTestId(page, 'tools-tabs-tab');
    await tabs.nth(1).click();
    await expect(byTestId(page, 'tools-panel')).toContainText('No files changed');
    await tabs.nth(2).click();
    await expect(byTestId(page, 'commands-empty')).toContainText(
      'Nothing running in the background',
    );
    await expectPlainWords(page);
    await expectAccessible(page);

    // Find it: the folder browser picks its new place, and the next turn runs there.
    await byTestId(page, 'thread-gone-find').click();
    await expect(byTestId(page, 'thread-gone-picking')).toContainText(
      'Where is the project’s folder now?',
    );
    await byTestId(page, 'folder-picker-folder').filter({ hasText: 'code' }).click();
    await byTestId(page, 'folder-picker-folder').filter({ hasText: 'notes' }).click();
    await expect(byTestId(page, 'folder-picker-folders')).toContainText('No folders in here.');
    await expectAccessible(page);
    await byTestId(page, 'folder-picker-use').click();
    await expect
      .poll(() => api.calls.filter((call) => call === 'project/folder/set'))
      .toHaveLength(1);
    expect(
      api.requests
        .filter((request) => request.method === 'project/folder/set')
        .map((request) => request.params),
    ).toEqual([{ project: PLAYGROUND.id, path: '/Users/ana/code/notes' }]);
    await expect(byTestId(page, 'thread-gone')).toHaveCount(0);
    await expect(byTestId(page, 'nav-gone')).toHaveCount(0);
    const composer = byTestId(page, 'thread-composer-input');
    await composer.fill('Make it bounce higher');
    await composer.press('Enter');
    await expect(byTestId(page, 'thread-message').last()).toContainText('Glad to help.');
    await expectPlainWords(page);
    expect(problems()).toEqual([]);
  });
}

test('Remove the project puts its threads in the archive, once the person confirms', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  const api = await fakeApi(page, { gone: true });
  await openLost(page);
  const project = byTestId(page, 'nav-project').filter({ hasText: PLAYGROUND.name });
  await expect(project).toHaveCount(1);
  const thread = byTestId(page, 'nav-thread').filter({ hasText: 'Sketch a bouncing ball' });
  await expect(thread).toHaveCount(1);

  await byTestId(page, 'nav-gone-remove').click();
  const confirm = byTestId(page, 'nav-gone-confirm');
  await expect(confirm).toContainText('Remove the project?');
  await expect(confirm).toContainText('Its threads go to the archive, with their history.');
  // Made as it opens, its code loaded late: focus still lands on its close button, named.
  await expect(byTestId(page, 'nav-gone-confirm-close')).toBeFocused();
  await expectAccessible(page);
  await byTestId(page, 'nav-gone-confirm-cancel').click();
  expect(api.calls).not.toContain('project/remove');

  await byTestId(page, 'nav-gone-remove').click();
  await byTestId(page, 'nav-gone-confirm-ok').click();
  await expect(project).toHaveCount(0);
  expect(
    api.requests
      .filter((request) => request.method === 'project/remove')
      .map((request) => request.params),
  ).toEqual([{ project: PLAYGROUND.id }]);
  // Its thread went to the archive with it: the side nav lists it no more.
  await expect(thread).toHaveCount(0);
  await expectPlainWords(page);
  expect(problems()).toEqual([]);
});
