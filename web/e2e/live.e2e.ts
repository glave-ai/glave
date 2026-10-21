import { expect, test } from '@playwright/test';

import { HELLO, LEFT_ALONE, TOUCH } from './fake-model';
import { LIVE, byTestId, expectAccessible, served, signInCode, watchForProblems } from './support';

/**
 * A real glave, its model the fake service of fake-model.ts, asking before commands run: what
 * one tab does in a thread, another tab open on it sees as it happens, the approval a turn
 * waits for among it, and either tab may answer.
 */
test('a second tab sees a turn and its approval as they happen, and answers it', async ({
  browser,
}) => {
  const context = await browser.newContext();
  const [first, second] = [await context.newPage(), await context.newPage()];
  const problems = [watchForProblems(first), watchForProblems(second)];
  const live = served(LIVE);

  await first.goto(`${live}/#code=${await signInCode(LIVE)}`);
  const prompt = byTestId(first, 'home-composer-input');
  await prompt.fill('Hi');
  await prompt.press('Enter');
  await expect(first).toHaveURL(/\/t\/[0-9a-f-]+$/u);
  await expect(byTestId(first, 'thread-message').last()).toContainText(HELLO);

  // The second tab opens the thread the first started, and follows it from then on.
  const thread = new URL(first.url()).pathname;
  await second.goto(`${live}${thread}#code=${await signInCode(LIVE)}`);
  await expect(byTestId(second, 'thread-message').last()).toContainText(HELLO);

  const reply = byTestId(first, 'thread-composer-input');
  await reply.fill('Please touch a file');
  await reply.press('Enter');
  const asked = byTestId(second, 'thread-message').filter({ hasText: 'Please touch a file' });
  await expect(asked).toBeVisible();
  const card = byTestId(second, 'thread-approval');
  await expect(card).toContainText('Needs you');
  await expect(card.locator('code')).toHaveText(TOUCH);
  for (const colorScheme of ['light', 'dark'] as const) {
    await second.emulateMedia({ colorScheme });
    await expectAccessible(second);
  }

  // Answered in the second tab, the turn goes on, and both tabs see how.
  await byTestId(second, 'thread-approval-deny').click();
  for (const page of [first, second]) {
    await expect(byTestId(page, 'thread-message').last()).toContainText(LEFT_ALONE);
  }
  // The first tab saw the decision too, in the turn's steps once it ended.
  await expect(byTestId(first, 'thread-approval-decided')).toHaveText('Denied');
  for (const seen of problems) {
    expect(seen()).toEqual([]);
  }
  await context.close();
});
