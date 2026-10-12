import { expect, test } from '@playwright/test';

import { NEW_THREAD, THREAD, fakeApi } from './fake-api';
import { byTestId, expectAccessible, shownOpacity, watchForProblems, SIGNED_IN } from './support';

test('without a sign-in, the app says how to get one', async ({ page }) => {
  await fakeApi(page);
  await page.goto('/');
  await expect(page).toHaveURL(/\/sign-in$/u);
  await expect(byTestId(page, 'sign-in')).toContainText('glave serve --http');
  await expectAccessible(page);
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the link signs in, leaves the address bar clean and lists the threads (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    await page.goto(`/${SIGNED_IN}`);
    await expect(byTestId(page, 'nav-thread').first()).toContainText('Make the tests pass');
    expect(page.url()).not.toContain('token');
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('a thread shows its history and streams a reply to a new message', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-steps-summary')).toHaveText('Ran 4 steps');
  await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
  await expectAccessible(page);
  const reply = byTestId(page, 'thread-composer-input');
  await reply.fill('Thanks');
  await reply.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Glad to help.');
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`tool calls in a row fold into steps that open to each call (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const summary = byTestId(page, 'thread-steps-summary');
    const receipts = byTestId(page, 'thread-receipt');
    await expect(summary).toHaveText('Ran 4 steps');
    await expect(receipts.first()).toBeHidden();
    // Left-aligned with the reply's text, not indented.
    const steps = await byTestId(page, 'thread-steps').boundingBox();
    const reply = await byTestId(page, 'thread-message').last().boundingBox();
    expect(Math.round(steps?.x ?? -1)).toBe(Math.round(reply?.x ?? 0));
    // A disclosure: the keyboard opens it, and each call keeps its tool and target.
    await summary.focus();
    await page.keyboard.press('Enter');
    await expect(receipts).toHaveCount(4);
    await expect(receipts.first()).toBeVisible();
    await expect(receipts.nth(0)).toContainText('bash');
    await expect(receipts.nth(0)).toContainText('cargo test');
    await expect(receipts.nth(1)).toContainText('edit');
    await expect(receipts.nth(1)).toContainText('src/parse.rs');
    await expect(receipts.nth(2)).toContainText('write');
    await expect(receipts.nth(2)).toContainText('tests/empty.rs');
    await expectAccessible(page);
    await page.keyboard.press('Enter');
    await expect(receipts.first()).toBeHidden();
  });

  test(`each message has a copy button, shown on hover and focus, that copies its text (${colorScheme})`, async ({
    page,
    context,
  }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const messages = byTestId(page, 'thread-message');
    await expect(messages.last()).toContainText('They pass now.');
    await expect(byTestId(page, 'message-copy')).toHaveCount(2);
    const copy = messages.last().getByTestId('message-copy');
    // Hidden until the pointer is over the message, or focus is in it.
    await page.mouse.move(0, 0);
    await expect.poll(() => shownOpacity(copy)).toBe(0);
    await copy.focus();
    await expect.poll(() => shownOpacity(copy)).toBe(1);
    await expectAccessible(page);
    await copy.press('Enter');
    await expect(copy).toHaveAttribute('aria-label', 'Copied');
    await expect(page.locator('[aria-live]', { hasText: 'Copied' })).toHaveCount(1);
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toContain('They pass now.');
    await messages.first().hover();
    await expect.poll(() => shownOpacity(messages.first().getByTestId('message-copy'))).toBe(1);
  });
}

test.describe('on a touch screen', () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test('the copy button always shows, since nothing hovers', async ({ page }) => {
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const copy = byTestId(page, 'thread-message').last().getByTestId('message-copy');
    await expect(copy).toBeAttached();
    await expect.poll(() => shownOpacity(copy)).toBe(1);
  });
});

test('a new chat starts a thread and sends the first message', async ({ page }) => {
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Plan the launch');
  await prompt.press('Enter');
  await expect(page).toHaveURL(new RegExp(`/t/${NEW_THREAD}$`, 'u'));
  await expect(byTestId(page, 'thread-message').first()).toContainText('Plan the launch');
  expect(api.calls).toContain('thread/start');
});

test('tasks, memory and settings open from the side nav', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'nav-tasks').click();
  await expect(page.getByRole('heading', { name: 'Tasks' })).toBeVisible();
  await expectAccessible(page);
  await byTestId(page, 'nav-memory').click();
  await expect(page.getByRole('heading', { name: 'Memory' })).toBeVisible();
  await expectAccessible(page);
  await byTestId(page, 'nav-settings').click();
  await expect(byTestId(page, 'settings-server')).toContainText('glave 0.0.0');
  await expectAccessible(page);
});
