import type { Page } from '@playwright/test';

/** A page with `glaveStored` in it (`storedEvents`), as a fake's page script sees `window`. */
export interface Storing {
  /** The `thread/event` notification of `type` with `data`, recorded in `thread` at `seq`. */
  readonly glaveStored: (
    thread: string,
    seq: number,
    at: string,
    type: string,
    data: unknown,
  ) => string;
}

/**
 * Puts `glaveStored` in the page, before any script of its own runs, for a fake's stream to
 * notify what it records as the server does. The order init scripts run in is not defined, so
 * a fake looks it up once its stream records something, as it does its gates.
 */
export async function storedEvents(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const glaveStored: Storing['glaveStored'] = (thread, seq, at, type, data) =>
      JSON.stringify({
        jsonrpc: '2.0',
        method: 'thread/event',
        params: {
          thread,
          event: {
            glave: 1,
            id: `01900000-0000-7000-8000-${String(seq).padStart(12, '0')}`,
            thread,
            seq,
            at,
            type,
            v: 1,
            data,
          },
        },
      });
    Object.assign(window, { glaveStored });
  });
}
