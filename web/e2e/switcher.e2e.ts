import type { OrganizationInfo } from '@glave/client';
import type { Page, Route } from '@playwright/test';

import { fakeApi } from './fake-api';
import { posted } from './fake-records';
import { OWN_ORGANIZATION } from './fake-signin';
import { ADMIN_WORDS, SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

/** The team Dana Lee owns, which the person joined by an invite. */
const TEAM = '01900000-0000-7000-8000-0000000000a2';

/**
 * A person in two organizations, their own and Dana Lee's team, over whatever `fakeApi`
 * answers: `organization/list` says which one each request works in, by the organization its
 * `Glave-Organization` header names, else their own. Every request's header is kept.
 */
async function inTwo(page: Page): Promise<{ readonly named: readonly (string | null)[] }> {
  const named: (string | null)[] = [];
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const header = (await route.request().headerValue('glave-organization')) ?? null;
    named.push(header);
    if (message.method !== 'organization/list') {
      await route.fallback();
      return;
    }
    const current = header ?? OWN_ORGANIZATION;
    const organizations: OrganizationInfo[] = [
      { id: OWN_ORGANIZATION, people: 1, own: true, role: 'owner', current: current !== TEAM },
      { id: TEAM, people: 3, role: 'member', owner: 'Dana Lee', current: current === TEAM },
    ];
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result: { organizations } }),
    });
  });
  return { named };
}

test('A person in two organizations switches from their own to a team and back from their menu', async ({
  page,
}) => {
  await fakeApi(page);
  const asked = await inTwo(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'nav-heading')).toHaveText('glave');
  await byTestId(page, 'person').click();
  const places = byTestId(page, 'person-switch');
  await expect(places).toHaveCount(2);
  await expect(places.first()).toContainText('Personal');
  await expect(places.first()).toContainText('Just you');
  await expect(places.first()).toHaveAttribute('aria-current', 'page');
  await expect(places.last()).toContainText("Dana Lee's team");
  await expect(places.last()).toContainText('3 people');
  await expect(byTestId(page, 'person-menu')).not.toContainText(ADMIN_WORDS);
  await expectAccessible(page);

  // The team: the page starts again there, and every request names it.
  await places.last().click();
  await expect(byTestId(page, 'nav-heading')).toHaveText("Dana Lee's team");
  expect(asked.named.at(-1)).toBe(TEAM);
  await byTestId(page, 'person').click();
  await expect(places.last()).toHaveAttribute('aria-current', 'page');

  // Back to their own: no request names one.
  await places.first().click();
  await expect(byTestId(page, 'nav-heading')).toHaveText('glave');
  expect(asked.named.at(-1)).toBeNull();
});

test('A person in one organization has nothing to switch between', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'nav-settings')).toBeVisible();
  await expect(byTestId(page, 'person-switch')).toHaveCount(0);
});
