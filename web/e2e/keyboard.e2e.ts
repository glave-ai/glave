import { type Page, expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { byTestId, expectAccessible } from './support';

const SIGNED_IN = '#token=glave_local_test';

/** Presses Tab until the element with `testId` has focus, as someone on a keyboard would. */
async function tabTo(page: Page, testId: string, most = 80): Promise<void> {
  for (let presses = 0; presses < most; presses += 1) {
    const focused = await page.evaluate(() => document.activeElement?.getAttribute('data-testid'));
    if (focused === testId) {
      return;
    }
    await page.keyboard.press('Tab');
  }
  throw new Error(`Tab never reached ${testId}`);
}

async function widthOf(page: Page, testId: string): Promise<number> {
  return Math.round((await byTestId(page, testId).boundingBox())?.width ?? 0);
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
});

test('the side nav resizes and collapses from its edge by keyboard', async ({ page }) => {
  const nav = byTestId(page, 'workspace-nav');
  const before = await widthOf(page, 'workspace-nav');
  await tabTo(page, 'workspace-handle');
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => widthOf(page, 'workspace-nav')).toBe(before + 32);
  await page.keyboard.press('ArrowLeft');
  await expect.poll(() => widthOf(page, 'workspace-nav')).toBe(before + 16);
  await page.keyboard.press('Enter');
  await expect(nav).toBeHidden();
  await expect(byTestId(page, 'workspace-handle')).toBeFocused();
  await expectAccessible(page);
  await page.keyboard.press('Enter');
  await expect(nav).toBeVisible();
  await expect.poll(() => widthOf(page, 'workspace-nav')).toBe(before + 16);
});

test('the tools panel resizes and collapses from its edge by keyboard', async ({ page }) => {
  const panel = byTestId(page, 'tools-panel');
  await tabTo(page, 'tools-handle');
  await page.keyboard.press('Enter');
  await expect(panel).toBeVisible();
  const before = await widthOf(page, 'tools-panel');
  await page.keyboard.press('ArrowLeft');
  await expect.poll(() => widthOf(page, 'tools-panel')).toBe(before + 16);
  await page.keyboard.press('Home');
  await expect.poll(() => widthOf(page, 'tools-panel')).toBe(280);
  await expectAccessible(page);
  await page.keyboard.press('Enter');
  await expect(panel).toBeHidden();
});

test('⌘B or Ctrl+B shows and hides the side nav, ⌘. or Ctrl+. the tools', async ({ page }) => {
  const nav = byTestId(page, 'workspace-nav');
  const panel = byTestId(page, 'tools-panel');
  await expect(byTestId(page, 'workspace-handle')).toHaveAttribute(
    'aria-keyshortcuts',
    /^(Meta|Control)\+B$/u,
  );
  await page.keyboard.press('ControlOrMeta+b');
  await expect(nav).toBeHidden();
  await page.keyboard.press('ControlOrMeta+b');
  await expect(nav).toBeVisible();
  await byTestId(page, 'thread-composer-input').focus();
  await page.keyboard.press('ControlOrMeta+.');
  await expect(panel).toBeVisible();
  await page.keyboard.press('ControlOrMeta+.');
  await expect(panel).toBeHidden();
});

test('a message is written and sent by keyboard alone', async ({ page }) => {
  await tabTo(page, 'thread-composer-input');
  await page.keyboard.type('Thanks');
  await page.keyboard.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Hello from Hope.');
});
