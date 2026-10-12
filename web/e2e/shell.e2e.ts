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
    await byTestId(page, 'tools-handle').press('Enter');
    await expect(byTestId(page, 'task').first()).toContainText('Nightly dependency audit');
    await expectAccessible(page);
    await byTestId(page, 'tools-tabs-tab').nth(1).click();
    await byTestId(page, 'file').first().click();
    await expect(byTestId(page, 'file-change').first()).toContainText('fn parse');
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('the side panels resize by their edge, collapse past it, and are remembered', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  // No toggle buttons on a wide screen: the edges and the shortcuts do it.
  await expect(byTestId(page, 'thread-tools')).toHaveCount(0);
  await expect(byTestId(page, 'page-menu')).toHaveCount(0);
  const panel = byTestId(page, 'tools-panel');
  const edge = byTestId(page, 'tools-handle');
  await expect(panel).toBeHidden();
  await expect(edge).toBeVisible();
  await expect(edge).toHaveAttribute('aria-valuenow', '0');
  // Dragging the collapsed edge toward the start brings the panel out.
  const box = await edge.boundingBox();
  const y = (box?.y ?? 0) + 200;
  const x = (box?.x ?? 0) + 4;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - 200, y, { steps: 4 });
  await page.mouse.move(x - 400, y, { steps: 4 });
  await page.mouse.up();
  await expect(panel).toBeVisible();
  await expect.poll(async () => Math.round((await panel.boundingBox())?.width ?? 0)).toBe(400);
  // Dragged back past half its minimum, it collapses to the slim edge.
  const open = await edge.boundingBox();
  await page.mouse.move((open?.x ?? 0) + 4, y);
  await page.mouse.down();
  await page.mouse.move((open?.x ?? 0) + 300, y, { steps: 6 });
  await page.mouse.up();
  await expect(panel).toBeHidden();
  await expect(edge).toBeVisible();
  // The side nav's edge: the same, and both are remembered after a reload.
  const nav = byTestId(page, 'workspace-nav');
  const navEdge = byTestId(page, 'workspace-handle');
  const navBox = await navEdge.boundingBox();
  await page.mouse.move((navBox?.x ?? 0) + 4, y);
  await page.mouse.down();
  await page.mouse.move((navBox?.x ?? 0) + 60, y, { steps: 4 });
  await page.mouse.up();
  const navWidth = Math.round((await nav.boundingBox())?.width ?? 0);
  await page.reload();
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  await expect.poll(async () => Math.round((await nav.boundingBox())?.width ?? 0)).toBe(navWidth);
  await expect(panel).toBeHidden();
  const navStart = await navEdge.boundingBox();
  await page.mouse.move((navStart?.x ?? 0) + 4, y);
  await page.mouse.down();
  await page.mouse.move(10, y, { steps: 6 });
  await page.mouse.up();
  await expect(nav).toBeHidden();
  await expect(navEdge).toHaveAttribute('aria-valuenow', '0');
  await page.reload();
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  await expect(nav).toBeHidden();
  await expect(navEdge).toBeVisible();
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
  await expect(byTestId(page, 'workspace-handle')).toHaveCount(0);
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
