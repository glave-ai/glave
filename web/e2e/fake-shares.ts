import type { Page, Route } from '@playwright/test';
import type { ShareInfo } from '@glave/client';

import { discovered } from './fake-signin';
import { posted } from './fake-records';

/** The link a share is given, at a name of its own under the share domain. */
export const LINK = 'http://abcdefghijklmnopqrstuvwxyz.localhost:4180/';

/**
 * Answers as a server with a share domain does, for a page on the fake API (set up after it):
 * it offers sharing with anyone, and keeps the one share it is asked to make, of any file.
 */
export async function fakeShares(page: Page): Promise<void> {
  let kept: ShareInfo | null = null;
  const answered = (method: string, params: Record<string, unknown>): unknown => {
    switch (method) {
      case 'server/discover':
        return { ...discovered(), shares: ['anyone'] };
      case 'share/get':
        return kept === null ? {} : { share: kept };
      case 'share/set': {
        const sha256 = typeof params['sha256'] === 'string' ? params['sha256'] : 'a'.repeat(64);
        kept =
          params['audience'] === undefined
            ? null
            : {
                audience: { kind: 'anyone' },
                link: LINK,
                shared_at: '2026-11-24T10:00:00.000Z',
                sha256,
                said: 'Shared with anyone who has the link',
              };
        return kept === null ? {} : { share: kept };
      }
      default:
        return undefined;
    }
  };
  await page.route('**/*', async (route: Route) => {
    // Anything but a call to the API has gone on already.
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const result = answered(message.method, message.params);
    await (result === undefined
      ? route.fallback()
      : route.fulfill({ json: { jsonrpc: '2.0', id: message.id, result } }));
  });
}
