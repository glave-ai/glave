import { THREAD, fakeApi } from './fake-api';
import { MORE, madeTheFiles } from './fake-made';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

/**
 * The name `browserName` saves a file called `name` under: WebKit on Linux, as CI runs it, saves
 * its spaces as underscores, whatever the page names it; Chromium keeps them.
 */
function savedAs(name: string, browserName: string): string {
  return browserName === 'webkit' && process.platform === 'linux'
    ? name.replaceAll(' ', '_')
    : name;
}

test('the files a chat made are cards: a sheet opens as a table in its panel, a PDF in a tab of its own', async ({
  page,
  browserName,
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
  const saving = page.waitForEvent('download');
  await byTestId(page, 'made-file-download').first().click();
  expect((await saving).suggestedFilename()).toBe(savedAs('Trip budget.xlsx', browserName));
  const open = byTestId(page, 'made-file-open');
  await expect(open.first()).toHaveAccessibleName('Open Trip budget');
  await open.first().click();
  await expect(byTestId(page, 'file-preview-title')).toHaveText('Trip budget');
  await expect(byTestId(page, 'file-preview').locator('table')).toContainText('Flights');
  // The PDF opens in a tab of its own, where the browser's viewer pages through it (a browser
  // with no viewer, as a headless one, saves it instead); the sheet stays in the panel.
  const tab = page.waitForEvent('popup');
  await open.last().click();
  await tab;
  await expect(byTestId(page, 'file-preview-title')).toHaveText('Trip budget');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(problems()).toEqual([]);
});

test('on a phone the whole card opens a file, and a PDF opens in a tab of its own', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await fakeApi(page);
  await madeTheFiles(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const cards = byTestId(page, 'made-file');
  await expect(cards).toHaveCount(3);
  const tab = page.waitForEvent('popup');
  await cards.nth(1).click();
  await tab;
  await expect(byTestId(page, 'file-preview')).toHaveCount(0);
  await cards.first().click();
  await expect(byTestId(page, 'file-preview').locator('table')).toContainText('Flights');
  await expect(byTestId(page, 'file-preview-download-thumb')).toBeVisible();
  await expectAccessible(page);
  expect(problems()).toEqual([]);
});
