import { type Page } from '@playwright/test';

import { PROJECT, THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

const SCREENS = [
  { name: 'a laptop', phone: false, size: { width: 1440, height: 900 } },
  { name: 'a phone', phone: true, size: { width: 390, height: 844 } },
] as const;

/** Opens Settings from the person's menu in the side nav, a drawer on a phone. */
async function openSettings(page: Page, phone: boolean): Promise<void> {
  if (phone) {
    await byTestId(page, 'page-menu').click();
  }
  await byTestId(page, 'person').click();
  await byTestId(page, 'nav-settings').click();
  await expect(page).toHaveURL(/\/settings$/u);
}

/** Fails unless the page shows the Chats, where Back leads and which has no Back. */
async function expectChats(page: Page): Promise<void> {
  await expect(page).toHaveURL(/\/$/u);
  await expect(page.getByRole('heading', { name: 'What should we work on?' })).toBeVisible();
  await expect(byTestId(page, 'page-back')).toHaveCount(0);
}

for (const colorScheme of ['light', 'dark'] as const) {
  for (const screen of SCREENS) {
    test(`Back from Memory opened from a chat leads back to that chat (${colorScheme}, ${screen.name})`, async ({
      page,
    }) => {
      const problems = watchForProblems(page);
      await page.emulateMedia({ colorScheme });
      await page.setViewportSize(screen.size);
      await fakeApi(page);
      await page.goto(`/t/${THREAD}${SIGNED_IN}`);
      await expect(byTestId(page, 'thread-message').first()).toBeVisible();
      // Memory lives in Settings: the person goes through it.
      await openSettings(page, screen.phone);
      await byTestId(page, 'settings-memory').click();
      await expect(page.getByRole('heading', { name: 'Memory' })).toBeVisible();
      const back = byTestId(page, 'page-back');
      await expect(back).toHaveText('Back');
      await expectAccessible(page);
      await back.click();
      await expect(page).toHaveURL(/\/settings$/u);
      await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
      // From the keyboard too.
      await byTestId(page, 'page-back').press('Enter');
      await expect(page).toHaveURL(new RegExp(`/t/${THREAD}$`, 'u'));
      await expect(byTestId(page, 'thread-message').first()).toBeVisible();
      expect(problems()).toEqual([]);
    });

    test(`Back from Memory opened by its address goes to the Chats (${colorScheme}, ${screen.name})`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme });
      await page.setViewportSize(screen.size);
      await fakeApi(page);
      await page.goto(`/memory${SIGNED_IN}`);
      await expect(page.getByRole('heading', { name: 'Memory' })).toBeVisible();
      await expectAccessible(page);
      await byTestId(page, 'page-back').click();
      await expectChats(page);
    });
  }
}

test('Back from adding a model account, then from Settings, leads back to the chat', async ({
  page,
}) => {
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').first()).toBeVisible();
  await openSettings(page, false);
  await byTestId(page, 'settings-add-account').click();
  await expect(page).toHaveURL(/\/connect$/u);
  await byTestId(page, 'connect-back').click();
  await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
  await byTestId(page, 'page-back').click();
  await expect(page).toHaveURL(new RegExp(`/t/${THREAD}$`, 'u'));
});

test('a project’s page, opened as glave opens it, leads back to the Chats', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/p/${PROJECT.id}${SIGNED_IN}`);
  await expect(byTestId(page, 'home-where-trigger')).toContainText('glave');
  await byTestId(page, 'page-back').click();
  await expectChats(page);
});
