import { type Locator, type Page, expect, test } from '@playwright/test';

import { TRUSTING, TRUST_ASK, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

/** What the card lists, in the plain words the server sent. */
const LISTED = [
  'Runs a tool server npx @acme/mcp',
  'Runs a hook before each command ./scripts/check.sh',
  'Lets glave approve actions without asking',
  'Turns off the sandbox',
];

/** The titles of the side nav's threads that say they wait for the person, in order. */
function waiting(page: Page): Locator {
  return byTestId(page, 'nav-thread')
    .filter({ has: byTestId(page, 'nav-thread-line') })
    .getByTestId('nav-thread-title');
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a project that brings its own tools asks in its thread and in the side nav, and trusting it is sent (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    const api = await fakeApi(page, { trust: true });
    await page.goto(`/t/${TRUSTING}${SIGNED_IN}`);
    const card = byTestId(page, 'thread-trust');
    await expect(card).toContainText('Needs you');
    await expect(card).toContainText('This project brings its own tools and settings');
    await expect(byTestId(page, 'thread-trust-item')).toHaveText(LISTED);
    await expect(card).not.toContainText(/\.mcp\.json|config\.toml|trust\.toml/u);
    await expect(byTestId(page, 'thread-trust-trust')).toHaveText('Trust this project');
    await expect(byTestId(page, 'thread-trust-not-now')).toHaveText('Not now');
    // Each project's thread that waits says so in its place: glave's, the latest active, first.
    await expect(waiting(page)).toHaveText(['Review the release notes', 'Search the Acme docs']);
    await expectAccessible(page);

    await byTestId(page, 'thread-trust-trust').click();
    await expect(byTestId(page, 'thread-trust-answered')).toHaveText('Trusted');
    await expect(byTestId(page, 'thread-trust-trust')).toHaveCount(0);
    const answers = api.requests.filter((request) => request.method === 'trust/answer');
    expect(answers.map((request) => request.params)).toEqual([
      { thread: TRUSTING, ask: TRUST_ASK.ask, answer: 'trust' },
    ]);
    await expect(waiting(page)).toHaveText(['Review the release notes']);
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('not now keeps the project off: the card rests, and nothing more is sent', async ({
  page,
}) => {
  const api = await fakeApi(page, { trust: true });
  await page.goto(`/t/${TRUSTING}${SIGNED_IN}`);
  await byTestId(page, 'thread-trust-not-now').click();
  await expect(byTestId(page, 'thread-trust-answered')).toHaveText('Kept off for now');
  await expect(byTestId(page, 'thread-trust-not-now')).toHaveCount(0);
  const answers = api.requests.filter((request) => request.method === 'trust/answer');
  expect(answers.map((request) => request.params['answer'])).toEqual(['not_now']);
  await expectAccessible(page);
});
