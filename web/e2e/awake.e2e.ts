import { fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`Settings keeps this computer awake while glave works until the person turns it off (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    const api = await fakeApi(page);
    await page.goto(`/settings${SIGNED_IN}`);
    const box = byTestId(page, 'awake-on-input');
    await expect(box).toBeChecked();
    await expect(byTestId(page, 'awake')).toContainText('closing the lid still puts it to sleep');
    await expectAccessible(page);
    await box.uncheck();
    await expect(box).not.toBeChecked();
    const sent = api.requests.filter(({ method }) => method === 'awake/set');
    expect(sent.map(({ params }) => params)).toEqual([{ on: false }]);
    // The server keeps the choice: it is what the page shows next time.
    await page.reload();
    await expect(box).not.toBeChecked();
  });
}
