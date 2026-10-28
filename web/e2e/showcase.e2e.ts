import {
  SHOWCASE,
  byTestId,
  expect,
  expectAccessible,
  expectDialogShown,
  test,
  watchForProblems,
} from './support';

// Glaze's showcase, a dev-only app served by its own glave (playwright.config.ts), read as the
// config reads the app's.
test.use({ baseURL: process.env[SHOWCASE.address] });

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the showcase passes axe in ${colorScheme}, under a strict CSP`, async ({
    page,
    browserName,
  }) => {
    test.skip(browserName === 'webkit', 'WebKit refuses a style attribute the showcase sets');
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    const response = await page.goto('/');
    const policy = response?.headers()['content-security-policy'] ?? '';
    expect(policy).toContain("script-src 'self'");
    expect(policy).toContain("require-trusted-types-for 'script'");
    expect(response?.headers()['cross-origin-opener-policy']).toBe('same-origin');
    await expect(byTestId(page, 'showcase-heading')).toBeVisible();
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

// Opened from the keyboard: Safari gives a button no focus when it is clicked, so focus has
// nothing to go back to after a click.
test('a dialog opens modally, closes on Escape and returns focus', async ({ page }) => {
  await page.goto('/');
  const opener = byTestId(page, 'showcase-open-dialog');
  await opener.focus();
  await opener.press('Enter');
  await expectDialogShown(page, 'showcase-dialog', [
    'showcase-dialog-title',
    'showcase-dialog-cancel',
    'showcase-dialog-save',
  ]);
  await expectAccessible(page);
  await page.keyboard.press('Escape');
  await expect(byTestId(page, 'showcase-dialog')).toBeHidden();
  await expect(opener).toBeFocused();
});

test('a confirmation shows its message and both its buttons', async ({ page }) => {
  await page.goto('/');
  await byTestId(page, 'showcase-open-confirm').click();
  await expectDialogShown(page, 'showcase-confirm', [
    'showcase-confirm-cancel',
    'showcase-confirm-ok',
  ]);
});

for (const size of [
  { width: 1280, height: 720 },
  { width: 375, height: 812 },
]) {
  test(`a long dialog scrolls only its body, its heading and actions in view (${String(size.width)} wide)`, async ({
    page,
  }) => {
    await page.setViewportSize(size);
    await page.goto('/');
    await byTestId(page, 'showcase-open-long').click();
    const dialog = byTestId(page, 'showcase-long');
    const body = byTestId(page, 'showcase-long-body');
    await expectDialogShown(page, 'showcase-long', ['showcase-long-done']);
    expect(await body.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(
      true,
    );
    expect(await dialog.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(
      false,
    );
    // Read to the end: the heading and Done are still there.
    await body.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await expectDialogShown(page, 'showcase-long', ['showcase-long-done']);
    for (const colorScheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme });
      await expectAccessible(page);
    }
  });
}

test('the composer sends, streams a reply and stops', async ({ page }) => {
  await page.goto('/');
  const input = byTestId(page, 'showcase-composer-input');
  await input.fill('Hello there');
  await input.press('Enter');
  await expect(byTestId(page, 'showcase-message').first()).toContainText('Hello there');
  await expect(byTestId(page, 'showcase-composer-send')).toBeVisible();
  await expect(byTestId(page, 'showcase-message').last()).toContainText('Anything else?');
});
