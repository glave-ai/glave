import { THREAD, fakeApi } from './fake-api';
import {
  SIGNED_IN,
  byTestId,
  expect,
  expectAccessible,
  hold,
  test,
  watchForProblems,
} from './support';

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

  test(`a screenshot and a PDF pasted into the box are shown and sent with the message (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    const { requests } = await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const input = byTestId(page, 'thread-composer-input');
    await expect(input).toBeVisible();

    // On their way, the files are shown and the message waits for them.
    const uploading = await hold(page, '/', 'file/upload');
    await input.evaluate(pasteScreenshotAndPdf);
    await uploading.sent;
    const chips = byTestId(page, 'thread-composer-file');
    await expect(chips).toHaveCount(2);
    await expect(chips.first()).toContainText('screenshot.png');
    await expect(chips.first().locator('img')).toHaveJSProperty('naturalWidth', 640);
    await expect(chips.last()).toContainText('terms.pdf');
    await expect(chips.first()).toHaveAttribute('aria-busy', 'true');
    await input.fill('What do these say?');
    await expect(byTestId(page, 'thread-composer-send')).toBeDisabled();
    uploading.letGo();
    await expect(byTestId(page, 'thread-composer-send')).toBeEnabled();
    await expectAccessible(page);

    await input.press('Enter');
    await expect(chips).toHaveCount(0);
    // Each is sent as soon as it is read, so the smaller may go first.
    const uploaded = requests.filter((request) => request.method === 'file/upload');
    expect(uploaded.map((request) => String(request.params['media_type'])).sort()).toEqual([
      'application/pdf',
      'image/png',
    ]);
    const sent = requests.filter((request) => request.method === 'turn/start').at(-1);
    expect(sent?.params['files']).toEqual([
      expect.objectContaining({ name: 'screenshot.png' }),
      expect.objectContaining({ name: 'terms.pdf' }),
    ]);
    expect(problems()).toEqual([]);
  });
}

/**
 * Pastes into `box` what a person copies from a screenshot tool and a folder: a 640 by 400
 * PNG, drawn here, and a PDF.
 */
async function pasteScreenshotAndPdf(box: HTMLElement): Promise<void> {
  const canvas = document.createElement('canvas');
  canvas.width = 640;
  canvas.height = 400;
  const context = canvas.getContext('2d');
  if (context !== null) {
    const shade = context.createLinearGradient(0, 0, 640, 400);
    shade.addColorStop(0, '#3b82f6');
    shade.addColorStop(1, '#f59e0b');
    context.fillStyle = shade;
    context.fillRect(0, 0, 640, 400);
  }
  const png = await new Promise<Blob | null>((done) => {
    canvas.toBlob(done, 'image/png');
  });
  const pasted = new DataTransfer();
  pasted.items.add(new File([png ?? ''], 'screenshot.png', { type: 'image/png' }));
  pasted.items.add(new File(['%PDF-1.7\n%%EOF\n'], 'terms.pdf', { type: 'application/pdf' }));
  box.dispatchEvent(
    new ClipboardEvent('paste', { clipboardData: pasted, bubbles: true, cancelable: true }),
  );
}
