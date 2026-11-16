import type { MadeFile } from '@glave/client';
import { type Page, type Route } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

/** The workbook the thread's turn made, as `thread/files` gives it. */
const BUDGET: MadeFile = {
  seq: 12,
  sha256: 'b'.repeat(64),
  name: 'Trip budget.xlsx',
  title: 'Trip budget',
  kind: 'spreadsheet',
  media_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  bytes: 12_288,
  kept: 'Kept with this chat',
};

/** Its sheets, as the server previews them. */
const PREVIEW = '## Budget\n| Item | Total |\n|---|---|\n| Flights | 780 |\n| Hotel | 580 |';

/** Answers `thread/files` with the budget, made in the thread's first turn, and its preview. */
async function madeABudget(page: Page): Promise<void> {
  const answers: Readonly<Record<string, unknown>> = {
    'thread/files': { made: [{ turn: 3, files: [BUDGET] }] },
    'file/preview': { markdown: PREVIEW },
  };
  await page.route('**/*', async (route: Route) => {
    const result = answers[route.request().headers()['glave-method'] ?? ''];
    if (result === undefined) {
      await route.fallback();
      return;
    }
    const { id } = route.request().postDataJSON() as { readonly id: unknown };
    await route.fulfill({ json: { jsonrpc: '2.0', id, result } });
  });
}

for (const [name, size] of [
  ['a laptop', { width: 1440, height: 900 }],
  ['a phone', { width: 390, height: 844 }],
] as const) {
  test(`a file the chat made is a card that opens in the panel (${name})`, async ({ page }) => {
    const problems = watchForProblems(page);
    await page.setViewportSize(size);
    await fakeApi(page);
    await madeABudget(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const card = byTestId(page, 'made-file');
    await expect(card).toHaveCount(1);
    await expect(byTestId(page, 'made-file-title')).toHaveText('Trip budget');
    await expect(byTestId(page, 'made-file-line')).toHaveText(
      'Spreadsheet · 12 kB · Kept with this chat',
    );
    await byTestId(page, 'made-file-open').click();
    const preview = byTestId(page, 'file-preview');
    await expect(preview).toBeVisible();
    await expect(byTestId(page, 'file-preview-title')).toHaveText('Trip budget');
    await expect(preview.locator('table')).toContainText('Flights');
    for (const colorScheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme });
      await expectAccessible(page);
    }
    expect(problems()).toEqual([]);
  });
}
