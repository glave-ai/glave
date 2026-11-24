import { THREAD, fakeApi } from './fake-api';
import { madeTheFiles } from './fake-made';
import { LINK, fakeShares } from './fake-shares';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

test('a file the chat made is shared at a link of its own, then stopped', async ({ page }) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await madeTheFiles(page);
  await fakeShares(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const share = byTestId(page, 'made-file-share');
  // Every file kept has Share; the one too large to keep has none.
  await expect(share).toHaveCount(2);
  await expect(share.first()).toHaveAccessibleName('Share Trip budget');
  await share.first().click();
  const dialog = byTestId(page, 'share');
  await expect(dialog).toBeVisible();
  await expect(byTestId(page, 'share-link')).toHaveCount(0);
  await byTestId(page, 'share-who').getByLabel('Anyone with the link').check();
  await byTestId(page, 'share-apply').click();
  await expect(byTestId(page, 'share-link')).toHaveValue(LINK);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  // Only you stops it: the link goes.
  await byTestId(page, 'share-who').getByLabel('Only you').check();
  await byTestId(page, 'share-apply').click();
  await expect(byTestId(page, 'share-link')).toHaveCount(0);
  await byTestId(page, 'share-close').click();
  await expect(dialog).toBeHidden();
  expect(problems()).toEqual([]);
});

test('without a share domain no file offers Share', async ({ page }) => {
  await fakeApi(page);
  await madeTheFiles(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'made-file')).toHaveCount(3);
  await expect(byTestId(page, 'made-file-share')).toHaveCount(0);
});
