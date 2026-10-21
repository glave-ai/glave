import { expect, test } from '@playwright/test';

import { TRUSTING, TRUST_ASK, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

/** What the card lists, in the plain words the server sent. */
const LISTED = [
  'Runs a tool server npx @acme/mcp',
  'Runs a hook before each command ./scripts/check.sh',
  'Lets glave approve actions without asking',
  'Turns off the sandbox',
];

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a project that brings its own tools asks in its thread and in Needs you, and trusting it is sent (${colorScheme})`, async ({
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
    await expect(byTestId(page, 'needs-you-thread')).toHaveText([
      'Search the Acme docs acme',
      'Review the release notes glave',
    ]);
    await expectAccessible(page);

    await byTestId(page, 'thread-trust-trust').click();
    await expect(byTestId(page, 'thread-trust-answered')).toHaveText('Trusted');
    await expect(byTestId(page, 'thread-trust-trust')).toHaveCount(0);
    const answers = api.requests.filter((request) => request.method === 'trust/answer');
    expect(answers.map((request) => request.params)).toEqual([
      { thread: TRUSTING, ask: TRUST_ASK.ask, answer: 'trust' },
    ]);
    await expect(byTestId(page, 'needs-you-thread')).toHaveText(['Review the release notes glave']);
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
