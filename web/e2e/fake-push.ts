import type { Page, Route } from '@playwright/test';
import type { PushDevice, PushInfo } from '@glave/client';

import { discovered } from './fake-signin';
import { type Answers, answerFrom, posted } from './fake-records';

/** The browsers a server that pushes keeps for the person, by endpoint. */
interface Pushed {
  readonly devices: Map<string, PushDevice>;
}

/** What the server says it pushes, the device at `endpoint` marked as the browser asking. */
function offered(pushed: Pushed, endpoint?: string | null): { readonly push: PushInfo } {
  const devices = [...pushed.devices].map(([at, device]) =>
    at === endpoint ? { ...device, current: true } : device,
  );
  return { push: { kind: 'offered', key: 'BPushKeyOfTheFakeServerE2e', devices } };
}

/** `server/discover` saying the server pushes, and `push/*` as a server that pushes answers. */
const KEPT_PUSH: Answers<Pushed> = {
  'server/discover': () => ({ ...discovered(), push: true }),
  'push/get': (params, pushed) => offered(pushed, params.endpoint),
  'push/subscribe': (params, pushed) => {
    pushed.devices.set(params.endpoint, {
      id: `0190000${String(pushed.devices.size)}-0000-7000-8000-000000000f01`,
      name: params.device,
      added: '2026-11-07T09:00:00.000Z',
    });
    return offered(pushed, params.endpoint);
  },
  'push/remove': (params, pushed) => {
    for (const [at, device] of pushed.devices) {
      if (device.id === params.device) {
        pushed.devices.delete(at);
      }
    }
    return offered(pushed);
  },
};

/**
 * Answers as a server that pushes does, for a page on the fake API (set up after it): it says
 * so, and keeps the browsers it is told of. The methods it answered, in order.
 */
export async function fakePush(page: Page): Promise<{ readonly calls: readonly string[] }> {
  const calls: string[] = [];
  const pushed: Pushed = { devices: new Map() };
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const result = answerFrom(KEPT_PUSH, message.method, message.params, pushed);
    if (result === undefined) {
      await route.fallback();
      return;
    }
    calls.push(message.method);
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result }),
    });
  });
  return { calls };
}
