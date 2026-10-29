import type { Page, Route } from '@playwright/test';

import { fakeApi } from './fake-api';
import { posted } from './fake-records';
import { discovered } from './fake-signin';
import { byTestId, expect, expectAccessible, test } from './support';

/**
 * A glave that takes sign-ups, over whatever `fakeApi` answers: it says so, and takes any
 * address. Every other call falls back to the routes before it.
 */
async function takingSignUps(page: Page): Promise<{ readonly asked: readonly unknown[] }> {
  const asked: unknown[] = [];
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const answered =
      message.method === 'server/discover'
        ? { ...discovered(), origin: 'https://glave.test', signin: ['link', 'passkey', 'signup'] }
        : message.method === 'signup/start'
          ? {}
          : undefined;
    if (answered === undefined) {
      await route.fallback();
      return;
    }
    asked.push(message.params);
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result: answered }),
    });
  });
  return { asked };
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`Someone new signs up with their email, then checks it (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    const glave = await takingSignUps(page);
    await page.goto('/sign-in');
    await byTestId(page, 'sign-in-sign-up').click();
    await expect(byTestId(page, 'sign-up-email')).toContainText('Get started with glave');
    await expectAccessible(page);
    await byTestId(page, 'sign-up-address').fill('ada@example.com');
    await byTestId(page, 'sign-up-send').click();
    await expect(byTestId(page, 'sign-up-sent')).toContainText('We sent a link to ada@example.com');
    await expectAccessible(page);
    expect(glave.asked).toContainEqual({ email: 'ada@example.com' });
  });

  test(`The emailed link opens the step that makes the passkey (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    await takingSignUps(page);
    await page.goto('/#signup=e2e-sign-up');
    await expect(page).toHaveURL(/\/sign-up$/u);
    await expect(byTestId(page, 'sign-up-passkey')).toContainText('Make your passkey');
    await expectAccessible(page);
  });
}
