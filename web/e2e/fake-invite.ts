import type { ShownInvite } from '@glave/client';
import type { Page, Route } from '@playwright/test';

import { posted } from './fake-records';

/** An invite's code that opens Acme's invite, and one that opens nothing. */
export const OPENS = '0192f5c600007000800000000000a0c0.e2e-invite';
export const SPENT = 'spent';

const SHOWN: ShownInvite = {
  organization: '0192f5c6-0000-7000-8000-000000000a0c',
  role: 'member',
  by: 'Dana Lee',
  joining: 'passkey',
};

/**
 * Answers what an invite's link opens (`invite/show`) and joining with it (`invite/accept`),
 * over whatever `fakeApi` answers: `OPENS` opens an invite, any other code nothing. Every other
 * call falls back to the routes before it.
 */
export async function fakeInvite(page: Page): Promise<{ readonly accepted: readonly unknown[] }> {
  const accepted: unknown[] = [];
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    if (message.method !== 'invite/show' && message.method !== 'invite/accept') {
      await route.fallback();
      return;
    }
    const opens = message.params['code'] === OPENS;
    if (opens && message.method === 'invite/accept') {
      accepted.push(message.params);
    }
    // Signed in, the person may accept it; signed out, they make a passkey.
    const signedIn = route.request().headers()['authorization'] !== undefined;
    const shown = { ...SHOWN, joining: signedIn ? 'accept' : 'passkey' };
    const reply = !opens
      ? { error: { code: -32035, message: 'this invite opens nothing: ask for a new one' } }
      : message.method === 'invite/show'
        ? { result: { shown } }
        : { result: { person: { id: 'p', role: 'member', you: true } } };
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, ...reply }),
    });
  });
  return { accepted };
}
