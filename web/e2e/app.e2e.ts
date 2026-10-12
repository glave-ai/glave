import { expect, test } from '@playwright/test';

import { NEW_THREAD, THREAD, fakeApi } from './fake-api';
import { byTestId, expectAccessible, watchForProblems } from './support';

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
    await page.goto('/#token=glave_local_test');
    await expect(byTestId(page, 'nav-thread').first()).toContainText('Make the tests pass');
    expect(page.url()).not.toContain('token');
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('a thread shows its history and streams a reply to a new message', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/t/${THREAD}#token=glave_local_test`);
  await expect(byTestId(page, 'thread-receipt').first()).toContainText('cargo test');
  await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
  await expectAccessible(page);
  const reply = byTestId(page, 'thread-composer-input');
  await reply.fill('Thanks');
  await reply.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Hello from Hope.');
});

test('a new chat starts a thread and sends the first message', async ({ page }) => {
  const api = await fakeApi(page);
  await page.goto('/#token=glave_local_test');
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Plan the launch');
  await prompt.press('Enter');
  await expect(page).toHaveURL(new RegExp(`/t/${NEW_THREAD}$`, 'u'));
  await expect(byTestId(page, 'thread-message').first()).toContainText('Plan the launch');
  expect(api.calls).toContain('thread/start');
});

test('tasks, memory and settings open from the side nav', async ({ page }) => {
  await fakeApi(page);
  await page.goto('/#token=glave_local_test');
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
