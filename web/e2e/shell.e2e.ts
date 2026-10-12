import { expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { byTestId, expectAccessible, watchForProblems } from './support';

const SIGNED_IN = '#token=glave_local_test';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the three panes pass axe with the tools open (${colorScheme})`, async ({ page }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-title')).toHaveText('Make the tests pass');
    await expect(byTestId(page, 'thread-space')).toHaveText('glave');
    await byTestId(page, 'thread-tools').click();
    await expect(byTestId(page, 'task').first()).toContainText('Nightly dependency audit');
    await expectAccessible(page);
    await byTestId(page, 'tools-tabs-tab').nth(1).click();
    await byTestId(page, 'file').first().click();
    await expect(byTestId(page, 'file-change').first()).toContainText('fn parse');
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('the tools panel opens and closes from the top bar, resizes, and is remembered', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const panel = byTestId(page, 'tools-panel');
  const toggle = byTestId(page, 'thread-tools');
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  await expect(panel).toBeHidden();
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await toggle.click();
  await expect(panel).toBeVisible();
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  const before = (await panel.boundingBox())?.width ?? 0;
  const handle = byTestId(page, 'tools-handle');
  await handle.focus();
  await handle.press('ArrowLeft');
  await handle.press('ArrowLeft');
  await expect.poll(async () => (await panel.boundingBox())?.width ?? 0).toBe(before + 32);
  await page.reload();
  await expect(panel).toBeVisible();
  await expect.poll(async () => (await panel.boundingBox())?.width ?? 0).toBe(before + 32);
  await byTestId(page, 'tools-close').click();
  await expect(panel).toBeHidden();
  await page.reload();
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  await expect(panel).toBeHidden();
});

test('the model picked is sent with the next turn, and each reply names its model', async ({
  page,
}) => {
  const api = await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').last()).toHaveAttribute('aria-label', 'Hope');
  const picker = byTestId(page, 'model-picker-trigger');
  await expect(picker).toContainText('Default model');
  await picker.click();
  await byTestId(page, 'model-picker-option').filter({ hasText: 'Hope' }).click();
  await expect(picker).toContainText('Hope');
  await expect(byTestId(page, 'approval-mode')).toContainText('Auto');
  const reply = byTestId(page, 'thread-composer-input');
  await reply.fill('Thanks');
  await reply.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Hello from Hope.');
  const turns = api.requests.filter((request) => request.method === 'turn/start');
  expect(turns).toEqual([
    {
      method: 'turn/start',
      params: expect.objectContaining({ thread: THREAD, prompt: 'Thanks', model: 'hope-12b' }),
    },
  ]);
});

test('the search keeps the threads whose title matches', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'nav-thread')).toHaveCount(4);
  await byTestId(page, 'nav-search').fill('release');
  await expect(byTestId(page, 'nav-thread')).toHaveText(['Review the release notes needs you']);
  await byTestId(page, 'nav-search').fill('zebra');
  await expect(byTestId(page, 'nav-thread')).toHaveCount(0);
  await expect(byTestId(page, 'nav-no-match')).toBeVisible();
  await byTestId(page, 'nav-search').fill('');
  await expect(byTestId(page, 'nav-thread')).toHaveCount(4);
});

test('a space group closes and opens, and + starts a thread in it', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  const toggle = byTestId(page, 'nav-space-toggle').first();
  await expect(toggle).toHaveText('glave');
  await expect(byTestId(page, 'nav-thread')).toHaveCount(4);
  const shown = byTestId(page, 'nav-thread').filter({ visible: true });
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(byTestId(page, 'nav-space-items').first()).toBeHidden();
  await expect(shown).toHaveText(['Dinner ideas for Saturday']);
  await toggle.click();
  await expect(shown).toHaveCount(4);
  await byTestId(page, 'nav-space-new').last().click();
  await expect(byTestId(page, 'home-space')).toHaveText('In Personal');
});

test('on a phone the side nav is a drawer and the tools a sheet', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  const nav = byTestId(page, 'workspace-nav');
  await expect(nav).toBeHidden();
  await byTestId(page, 'page-menu').click();
  await expect(nav).toBeVisible();
  await expect(nav).toHaveAttribute('aria-modal', 'true');
  await expectAccessible(page);
  await page.keyboard.press('Escape');
  await expect(nav).toBeHidden();
  await byTestId(page, 'thread-tools').click();
  const panel = byTestId(page, 'tools-panel');
  await expect(panel).toHaveAttribute('aria-modal', 'true');
  await expect(byTestId(page, 'tools-handle')).toHaveCount(0);
  expect((await panel.boundingBox())?.width).toBe(390);
  await expectAccessible(page);
  await page.keyboard.press('Escape');
  await expect(panel).toBeHidden();
});
