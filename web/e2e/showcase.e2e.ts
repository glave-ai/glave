import { expect, test } from '@playwright/test';

import { byTestId, expectAccessible, watchForProblems } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the showcase passes axe in ${colorScheme}, under a strict CSP`, async ({ page }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    const response = await page.goto('/glaze');
    const policy = response?.headers()['content-security-policy'] ?? '';
    expect(policy).toContain("script-src 'self'");
    expect(policy).toContain("require-trusted-types-for 'script'");
    expect(response?.headers()['cross-origin-opener-policy']).toBe('same-origin');
    await expect(byTestId(page, 'showcase-heading')).toBeVisible();
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('a dialog opens modally, closes on Escape and returns focus', async ({ page }) => {
  await page.goto('/glaze');
  const opener = byTestId(page, 'showcase-open-dialog');
  await opener.click();
  const title = byTestId(page, 'showcase-dialog-title');
  await expect(title).toBeVisible();
  await expectAccessible(page);
  await page.keyboard.press('Escape');
  await expect(title).toBeHidden();
  await expect(opener).toBeFocused();
});

test('the composer sends, streams a reply and stops', async ({ page }) => {
  await page.goto('/glaze');
  const input = byTestId(page, 'showcase-composer-input');
  await input.fill('Hello there');
  await input.press('Enter');
  await expect(byTestId(page, 'showcase-message').first()).toContainText('Hello there');
  await expect(byTestId(page, 'showcase-composer-send')).toBeVisible();
  await expect(byTestId(page, 'showcase-message').last()).toContainText('Anything else?');
});
