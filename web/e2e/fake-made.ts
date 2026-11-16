import type { MadeFile, RawEvent } from '@glave/client';
import { type Page, type Route } from '@playwright/test';

import { HISTORY } from './fake-api';

/** The workbook the thread's turn made, as `thread/files` gives it. */
export const BUDGET: MadeFile = {
  kind: 'spreadsheet',
  name: 'Trip budget',
  file_name: 'Trip budget.xlsx',
  media_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  bytes: 12_288,
  sha256: 'b'.repeat(64),
  parts: '2 sheets',
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a spreadsheet: Trip budget (2 sheets)',
};

/** The PDF the same turn made. */
export const PLAN: MadeFile = {
  kind: 'pdf',
  name: 'Lisbon plan',
  file_name: 'Lisbon plan.pdf',
  media_type: 'application/pdf',
  bytes: 9,
  sha256: 'c'.repeat(64),
  parts: '1 page',
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a PDF: Lisbon plan (1 page)',
};

/** The budget's sheets, as the server previews them. */
const PREVIEW = '## Budget\n| Item | Total |\n|---|---|\n| Flights | 780 |\n| Hotel | 580 |';

/** How a turn's end names `file`, kept. */
function kept(file: MadeFile): unknown {
  return { blob: { sha256: file.sha256, bytes: file.bytes, media_type: file.media_type } };
}

/** The thread's history, its turn's end saying it made the two files. */
const MADE_HISTORY: readonly RawEvent[] = HISTORY.map((event) =>
  event.type === 'turn.ended'
    ? {
        ...event,
        data: {
          ...event.data,
          made: [
            { path: 'out/Trip budget.xlsx', bytes: BUDGET.bytes, kept: kept(BUDGET) },
            { path: 'Lisbon plan.pdf', bytes: PLAN.bytes, kept: kept(PLAN) },
          ],
        },
      }
    : event,
);

/** What the server answers about the files, by the method it is asked. */
const ANSWERS: Readonly<Record<string, unknown>> = {
  'thread/events': { events: MADE_HISTORY },
  'thread/turns': { events: MADE_HISTORY },
  'thread/files': { made: [{ turn: 3, files: [BUDGET, PLAN] }] },
  'file/preview': { preview: { markdown: PREVIEW } },
  // "%PDF-1.7", as the PDF's bytes.
  'file/read': {
    piece: { media_type: 'application/pdf', bytes: 9, offset: 0, data: 'JVBERi0xLjcK' },
  },
};

/**
 * Answers, after `fakeApi`, as the server would for the thread whose turn made a budget and a
 * plan: its history, the files, the budget's preview and the plan's bytes.
 */
export async function madeTheFiles(page: Page): Promise<void> {
  await page.route('**/*', async (route: Route) => {
    const result = ANSWERS[route.request().headers()['glave-method'] ?? ''];
    if (result === undefined) {
      await route.fallback();
      return;
    }
    const { id } = route.request().postDataJSON() as { readonly id: unknown };
    await route.fulfill({ json: { jsonrpc: '2.0', id, result } });
  });
}
