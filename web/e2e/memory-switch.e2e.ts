import { fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`The memory page lets the person turn memory off, and the server keeps it (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    const api = await fakeApi(page);
    await page.goto(`/memory${SIGNED_IN}`);
    const box = byTestId(page, 'memory-switch-on-input');
    await expect(box).toBeChecked();
    await expect(box).toBeEnabled();
    await expectAccessible(page);
    await box.uncheck();
    await expect(box).not.toBeChecked();
    const sent = api.requests.filter(({ method }) => method === 'memory/set');
    expect(sent.map(({ params }) => params)).toEqual([{ on: false }]);
    // The server keeps the choice: it is what the page shows next time.
    await page.reload();
    await expect(box).not.toBeChecked();
  });

  test(`Where memory is turned off for everyone, the switch shows off and can't be changed, with the server's sentence (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page, { remembering: { on: false, never: 'memory is off for this server' } });
    await page.goto(`/memory${SIGNED_IN}`);
    const box = byTestId(page, 'memory-switch-on-input');
    await expect(box).not.toBeChecked();
    await expect(box).toBeDisabled();
    await expect(byTestId(page, 'memory-switch')).toContainText('Memory is off for this server');
    await expectAccessible(page);
  });
}
