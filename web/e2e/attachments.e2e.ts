import { expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

/** A one-pixel PNG, as a screenshot would come. */
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  'base64',
);

for (const colorScheme of ['light', 'dark'] as const) {
  test(`files are added by picking and dropping them, shown, and sent with the message (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    const { requests } = await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-composer-attach')).toBeVisible();

    await byTestId(page, 'thread-composer-picker').setInputFiles([
      { name: 'screenshot.png', mimeType: 'image/png', buffer: PNG },
      { name: 'clip.mp4', mimeType: 'video/mp4', buffer: Buffer.from('not really a video') },
    ]);
    const chips = byTestId(page, 'thread-composer-file');
    await expect(chips).toHaveCount(2);
    // The image is shown as itself, from this computer; the video says why it was refused.
    await expect(chips.first()).toContainText('screenshot.png');
    await expect(chips.first().locator('img')).toHaveJSProperty('naturalWidth', 1);
    await expect(chips.last()).toContainText("glave can't use videos yet");
    await expectAccessible(page);
    await byTestId(page, 'thread-composer-file-remove').last().click();
    await expect(chips).toHaveCount(1);

    // Dropped on the composer, a file is added too.
    await byTestId(page, 'thread-composer').evaluate((composer) => {
      const dropped = new DataTransfer();
      dropped.items.add(new File(['- [ ] ship it'], 'notes.md', { type: 'text/markdown' }));
      composer.dispatchEvent(
        new DragEvent('dragover', { dataTransfer: dropped, bubbles: true, cancelable: true }),
      );
      composer.dispatchEvent(
        new DragEvent('drop', { dataTransfer: dropped, bubbles: true, cancelable: true }),
      );
    });
    await expect(chips).toHaveCount(2);
    await expect(chips.last()).toContainText('notes.md');

    const composer = byTestId(page, 'thread-composer-input');
    await composer.fill('What do these show?');
    await composer.press('Enter');
    await expect(chips).toHaveCount(0);
    const sent = requests.filter((request) => request.method === 'turn/start').at(-1);
    expect(sent?.params['files']).toEqual([
      expect.objectContaining({ name: 'screenshot.png' }),
      expect.objectContaining({ name: 'notes.md' }),
    ]);
    expect(problems()).toEqual([]);
  });
}
