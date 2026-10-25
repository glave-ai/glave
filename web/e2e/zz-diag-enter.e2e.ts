// Diagnostic for #699 only, never committed: Enter that reaches the page before its next task.
import { expect, test } from '@playwright/test';

import { fakeApi } from './fake-api';
import { SIGNED_IN, byTestId } from './support';

// A trace's snapshots wait for the page, which would let it run its next task before Enter.
test.use({ trace: 'off' });

test('Enter right after the name lands, before the page runs another task', async ({ page }) => {
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'nav-new-project').click();
  await page.getByRole('radio', { name: 'Empty' }).check();
  const name = byTestId(page, 'new-project-name');
  await name.focus();
  await expect(byTestId(page, 'new-project-create')).toBeDisabled();
  // The name lands, then the page stays busy, so Enter is waiting when it is free again.
  const busy = page.evaluate(() => {
    const input = document.querySelector<HTMLInputElement>('[data-testid="new-project-name"]');
    if (input === null) {
      return 'no field';
    }
    input.value = 'Ideas';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const disabled = document.querySelector<HTMLButtonElement>(
      '[data-testid="new-project-create"]',
    )?.disabled;
    const end = performance.now() + 400;
    while (performance.now() < end) {
      // busy
    }
    return `typed; Create disabled: ${String(disabled)}; focused: ${document.activeElement === input}`;
  });
  await page.waitForTimeout(150);
  await page.keyboard.press('Enter');
  console.log(await busy);
  await expect
    .poll(() => api.requests.filter(({ method }) => method === 'project/create').length, {
      timeout: 5000,
    })
    .toBe(1);
});
