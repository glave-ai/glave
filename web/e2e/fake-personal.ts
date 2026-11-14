import type { OrganizationInfo, ServerInfo } from '@glave/client';
import type { Page, Route } from '@playwright/test';

import { posted } from './fake-records';
import { OWN_ORGANIZATION, discovered } from './fake-signin';

/** The organization the person creates. */
const NURFZ = '01900000-0000-7000-8000-0000000000a3';

/**
 * A person signed up on a server that takes sign-ups, in their personal space, over whatever
 * `fakeApi` answers: `server/discover` says it takes no invites and that they may create an
 * organization; `organization/create` makes Nurfz, which `organization/list` then lists, and in
 * which, named by the `Glave-Organization` header, they may invite.
 */
export async function personal(page: Page): Promise<{ readonly created: readonly unknown[] }> {
  const created: unknown[] = [];
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const inNurfz = (await route.request().headerValue('glave-organization')) === NURFZ;
    const answer = (result: unknown): Promise<void> =>
      route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result }),
      });
    const nurfz: OrganizationInfo = { id: NURFZ, name: 'Nurfz', people: 1, role: 'owner' };
    switch (message.method) {
      case 'server/discover': {
        const said: ServerInfo = {
          ...discovered(),
          origin: 'https://glave.example',
          organizations: true,
          ...(inNurfz ? { invites: true } : { personal: true }),
        };
        await answer(said);
        return;
      }
      case 'organization/create':
        created.push(message.params);
        await answer({ organization: nurfz });
        return;
      case 'organization/list': {
        const own: OrganizationInfo = {
          id: OWN_ORGANIZATION,
          name: 'Personal',
          people: 1,
          own: true,
          role: 'owner',
          current: !inNurfz,
        };
        const theirs = created.length > 0 ? [own, { ...nurfz, current: inNurfz }] : [own];
        await answer({ organizations: theirs });
        return;
      }
      default:
        await route.fallback();
    }
  });
  return { created };
}
