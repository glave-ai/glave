import { type Page, expect, test } from '@playwright/test';

import { THREAD, type Request, fakeApi } from './fake-api';
import { byTestId, expectAccessible } from './support';

const SIGNED_IN = '#token=glave_local_test';

/**
 * Presses Tab (or Shift+Tab, `back`) until the element with `testId` has focus, as someone on
 * a keyboard would.
 */
async function tabTo(page: Page, testId: string, back = false, most = 80): Promise<void> {
  for (let presses = 0; presses < most; presses += 1) {
    const focused = await page.evaluate(() => document.activeElement?.getAttribute('data-testid'));
    if (focused === testId) {
      return;
    }
    await page.keyboard.press(back ? 'Shift+Tab' : 'Tab');
  }
  throw new Error(`Tab never reached ${testId}`);
}

async function widthOf(page: Page, testId: string): Promise<number> {
  return Math.round((await byTestId(page, testId).boundingBox())?.width ?? 0);
}

let requests: readonly Request[] = [];

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  requests = (await fakeApi(page)).requests;
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
  await expect(byTestId(page, 'thread-message').last()).toContainText('Glad to help.');
});

test('a model is searched, picked and pinned by keyboard alone', async ({ page }) => {
  await tabTo(page, 'model-picker-trigger');
  await page.keyboard.press('Enter');
  const search = byTestId(page, 'model-picker-search');
  await expect(search).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('PageDown');
  await expect(byTestId(page, 'model-picker-option').nth(11)).toBeInViewport();
  await page.keyboard.type('luna');
  await page.keyboard.press('Tab');
  await expect(byTestId(page, 'model-picker-pin')).toBeFocused();
  await expect(byTestId(page, 'model-picker-pin')).toHaveText('Pin OpenAI · GPT-6 Luna');
  await page.keyboard.press('Enter');
  await expect(byTestId(page, 'model-picker-pin')).toHaveText('Unpin OpenAI · GPT-6 Luna');
  await page.keyboard.press('Shift+Tab');
  await expect(search).toBeFocused();
  await page.keyboard.press('Enter');
  const trigger = byTestId(page, 'model-picker-trigger');
  await expect(trigger).toBeFocused();
  await expect(trigger).toContainText('OpenAI · GPT-6 Luna');
  await tabTo(page, 'thread-composer-input', true);
  await page.keyboard.type('Go on');
  await page.keyboard.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Glad to help.');
  const turn = requests.find((request) => request.method === 'turn/start');
  expect(turn?.params).toMatchObject({ prompt: 'Go on', model: 'openai/gpt-6-luna' });
});

test('the approval mode is chosen by keyboard and goes with the next turn', async ({ page }) => {
  const trigger = byTestId(page, 'approval-mode-trigger');
  await expect(trigger).toContainText('Auto');
  await tabTo(page, 'approval-mode-trigger');
  await page.keyboard.press('Enter');
  const ask = byTestId(page, 'approval-mode-option').first();
  await expect(ask).toBeVisible();
  await expectAccessible(page);
  // Opened from the keyboard, the menu focuses its first mode, the cautious one.
  await expect(ask).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(trigger).toContainText('Ask');
  await tabTo(page, 'thread-composer-input', true);
  await page.keyboard.type('Carefully');
  await page.keyboard.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Glad to help.');
  const turn = requests.find((request) => request.method === 'turn/start');
  expect(turn?.params).toMatchObject({ prompt: 'Carefully', approval: 'ask' });
});
