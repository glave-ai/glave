import type { MadeFile, RawEvent } from '@glave/client';
import { type Page, type Route } from '@playwright/test';

import { HISTORY, THREAD } from './fake-api';
import { DEFAULT_MODEL } from './fake-models';
import { event } from './fake-records';

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

/** A deck the same turn made, too large to keep, whose line says why. */
export const PHOTOS: MadeFile = {
  kind: 'deck',
  name: 'All the photos',
  file_name: 'All the photos.pptx',
  media_type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  bytes: 24_000_000,
  previews: false,
  kept: 'Not kept: larger than 10 MB, so it stays on the computer',
  sentence: 'Made a deck: All the photos',
};

/** How a turn's end names `file`, kept and read into `parts`. */
function kept(file: MadeFile, parts: unknown): Readonly<Record<string, unknown>> {
  const blob = { sha256: file.sha256, bytes: file.bytes, media_type: file.media_type };
  return { kept: { blob }, read: { parts } };
}

/** The turn after the one that made the files, which only talks: what moves would move it. */
function thanked(seq: number): readonly RawEvent[] {
  const model = { provider: 'e2e', name: DEFAULT_MODEL };
  const text = (role: string, said: string): RawEvent['data'] => ({
    role,
    parts: [{ type: 'text', text: said }],
  });
  const at = '2026-10-11T10:03:00.000Z';
  return [
    event(seq, 'turn.started', { model }, THREAD, at),
    event(seq + 1, 'message.added', text('user', 'Thanks, that works.'), THREAD, at),
    event(seq + 2, 'message.added', text('assistant', 'Glad it helps.'), THREAD, at),
    event(seq + 3, 'model.responded', { model, stop: 'end_turn' }, THREAD, at),
    event(seq + 4, 'turn.ended', { outcome: 'completed', model_calls: 1 }, THREAD, at),
  ];
}

/**
 * The thread's history: its first turn's end says it made the three files, one too large to keep,
 * and left one more on the computer, as the note under them says, and another turn follows it.
 */
const MADE_HISTORY: readonly RawEvent[] = [
  ...HISTORY.map((each) =>
    each.type === 'turn.ended'
      ? {
          ...each,
          data: {
            ...each.data,
            made: [
              { path: 'out/Trip budget.xlsx', bytes: BUDGET.bytes, ...kept(BUDGET, { sheets: 2 }) },
              { path: 'Lisbon plan.pdf', bytes: PLAN.bytes, ...kept(PLAN, { pages: 1 }) },
              { path: 'All the photos.pptx', bytes: PHOTOS.bytes, kept: 'too_large' },
            ],
            made_left: { kind: 'more', files: 1 },
          },
        }
      : each,
  ),
  ...thanked(HISTORY.length + 1),
];

/** What the turn's files leave out, in the server's words. */
export const MORE = '1 more file was made than a chat keeps: it stays on the computer';

/** What the server answers about the files, by the method it is asked. */
const ANSWERS: Readonly<Record<string, unknown>> = {
  'thread/events': { events: MADE_HISTORY },
  'thread/turns': { events: MADE_HISTORY },
  'thread/files': { made: [{ turn: 3, files: [BUDGET, PLAN, PHOTOS], note: MORE }] },
  // "%PDF-1.7", as the file's bytes.
  'file/read': {
    piece: { media_type: 'application/pdf', bytes: 9, offset: 0, data: 'JVBERi0xLjcK' },
  },
};

/**
 * Answers, after `fakeApi`, as the server would for the thread whose turn made a budget and a
 * plan: its history, the files, and their bytes.
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
