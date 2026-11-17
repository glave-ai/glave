import { THREAD, fakeApi } from './fake-api';
import { MORE, madeTheFiles } from './fake-made';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

test('the files a chat made are cards after its reply, each saved under its name', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await madeTheFiles(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const cards = byTestId(page, 'made-file');
  await expect(cards).toHaveCount(3);
  await expect(byTestId(page, 'made-file-title')).toHaveText([
    'Trip budget',
    'Lisbon plan',
    'All the photos',
  ]);
  await expect(byTestId(page, 'made-file-line').last()).toContainText(
    'Not kept: larger than 10 MB',
  );
  await expect(byTestId(page, 'made-file-line').first()).toHaveText(
    'Spreadsheet · 2 sheets · Kept with this chat',
  );
  await expect(byTestId(page, 'turn-made-note')).toHaveText(MORE);
  // Download saves the file under its name, in Chromium and in WebKit (Safari's engine).
  const download = byTestId(page, 'made-file-download');
  await expect(download.first()).toHaveAccessibleName('Download Trip budget');
  const saving = page.waitForEvent('download');
  await download.first().click();
  expect((await saving).suggestedFilename()).toBe('Trip budget.xlsx');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(problems()).toEqual([]);
});

test('on a phone the cards fit the screen, and a file still saves under its name', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await fakeApi(page);
  await madeTheFiles(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const cards = byTestId(page, 'made-file');
  await expect(cards).toHaveCount(3);
  const width = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(width).toBeLessThanOrEqual(390);
  const saving = page.waitForEvent('download');
  await byTestId(page, 'made-file-download').last().click();
  expect((await saving).suggestedFilename()).toBe('Lisbon plan.pdf');
  await expectAccessible(page);
  expect(problems()).toEqual([]);
});
