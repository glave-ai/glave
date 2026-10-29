import type { InviteInfo, PersonInfo, ServerInfo } from '@glave/client';
import type { Page, Route } from '@playwright/test';

import { discovered } from './fake-signin';

/** The organization's people as its Owner, Dana, sees them. */
const DANA: PersonInfo = {
  id: '01900000-0000-7000-8000-0000000000d1',
  name: 'Dana Lee',
  email: 'dana@example.com',
  role: 'owner',
  you: true,
};
const SAM: PersonInfo = {
  id: '01900000-0000-7000-8000-0000000000d2',
  name: 'Sam Ortiz',
  email: 'sam@example.com',
  role: 'admin',
  roles: ['owner', 'admin', 'member'],
  removable: true,
};
const PRIYA: PersonInfo = {
  id: '01900000-0000-7000-8000-0000000000d3',
  name: 'Priya Nair',
  email: 'priya@example.com',
  role: 'member',
  roles: ['owner', 'admin', 'member'],
  removable: true,
};

/** An invite Sam made, a link with no email, made a day before `now`. */
function samsInvite(now: number): InviteInfo {
  return {
    id: '01900000-0000-7000-8000-0000000000e1',
    role: 'admin',
    by: { kind: 'person', id: SAM.id },
    made: new Date(now - 86_400_000).toISOString(),
    expires: new Date(now + 6 * 86_400_000 - 3_600_000).toISOString(),
  };
}

/** The code of the invite the page makes. */
export const INVITE_CODE = 'e2e-invite-code';

/** What the server keeps of the organization while a test runs. */
interface Kept {
  people: PersonInfo[];
  invites: InviteInfo[];
}

/** The answer to an admin call, or `undefined` for one this fake leaves to the next. */
function answer(method: string, params: Record<string, unknown>, kept: Kept): unknown {
  switch (method) {
    case 'people/list':
      return { people: kept.people };
    case 'invite/list':
      return { invites: kept.invites };
    case 'role/set': {
      const role = params['role'] as PersonInfo['role'];
      kept.people = kept.people.map((each) =>
        each.id === params['person'] ? { ...each, role } : each,
      );
      return { person: kept.people.find((each) => each.id === params['person']) };
    }
    case 'person/remove':
      kept.people = kept.people.filter((each) => each.id !== params['person']);
      return {};
    case 'invite/revoke':
      kept.invites = kept.invites.filter((each) => each.id !== params['invite']);
      return {};
    case 'invite/create': {
      const now = Date.now();
      const email = params['email'];
      const invite: InviteInfo = {
        id: '01900000-0000-7000-8000-0000000000e2',
        role: params['role'] as InviteInfo['role'],
        by: { kind: 'person', id: DANA.id },
        made: new Date(now).toISOString(),
        expires: new Date(now + 7 * 86_400_000).toISOString(),
        ...(typeof email === 'string' ? { email } : {}),
      };
      kept.invites = [...kept.invites, invite];
      return { invite, code: INVITE_CODE };
    }
    default:
      return undefined;
  }
}

/**
 * Answers the admin calls of an organization's Owner, over whatever `fakeApi` answers: the
 * server says there is an organization and the caller owns it, at a public origin, and keeps
 * its people and invites as the page changes them. Every other call falls back to the routes
 * before it.
 */
export async function fakeAdmin(page: Page): Promise<{ readonly sent: readonly unknown[] }> {
  const kept: Kept = { people: [DANA, SAM, PRIYA], invites: [samsInvite(Date.now())] };
  const sent: unknown[] = [];
  const server: ServerInfo = {
    ...discovered(),
    origin: 'https://glave.example',
    exists: { organization: true, workers: false },
    role: 'owner',
  };
  await page.route('**/*', async (route: Route) => {
    const request = route.request();
    if (request.method() !== 'POST') {
      await route.fallback();
      return;
    }
    const message = request.postDataJSON() as {
      id: number;
      method: string;
      params: Record<string, unknown>;
    };
    const answered =
      message.method === 'server/discover' ? server : answer(message.method, message.params, kept);
    if (answered === undefined) {
      await route.fallback();
      return;
    }
    sent.push({ method: message.method, params: message.params });
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result: answered }),
    });
  });
  return { sent };
}
