import { type Locator, expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { ANSWER, THINKING, THINKING_PROMPT, letThink, thinkingTurn } from './fake-thinking';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

/** Whether a reasoning block's disclosure is open. */
async function isOpen(reasoning: Locator): Promise<boolean> {
  return reasoning.locator('details').evaluate((details) => (details as HTMLDetailsElement).open);
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`glave shows it is working at once, then the model's thinking, then the reply (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await thinkingTurn(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
    const composer = byTestId(page, 'thread-composer-input');
    await composer.fill(THINKING_PROMPT);
    await composer.press('Enter');

    // Before the server has recorded anything: the message, and glave working under it.
    const working = byTestId(page, 'thread-working');
    await expect(byTestId(page, 'thread-message').last()).toContainText(THINKING_PROMPT);
    await expect(working).toBeVisible();
    await expect(byTestId(page, 'thread-working-label')).toHaveText('Thinking…');
    await expectAccessible(page);
    await letThink(page, 'start');
    await expect(byTestId(page, 'thread-message').last()).toContainText(THINKING_PROMPT);
    await expect(working).toBeVisible();

    // The model's reasoning streams in, open, in the working state's place.
    await letThink(page, 'think');
    const reasoning = byTestId(page, 'thread-reply').getByTestId('thread-reasoning');
    await expect(reasoning.getByText(THINKING.join(''))).toBeVisible();
    await expect(working).toHaveCount(0);
    await expect(reasoning.getByTestId('thread-reasoning-summary')).toHaveText('Thinking…');
    expect(await isOpen(reasoning)).toBe(true);
    await expectAccessible(page);

    // The answer folds it away; once recorded, what was thought can still be opened.
    await letThink(page, 'answer');
    const reply = byTestId(page, 'thread-message').last();
    await expect(reply).toContainText(ANSWER.join(''));
    await expect(byTestId(page, 'thread-composer-stop')).toHaveCount(0);
    const kept = reply.getByTestId('thread-reasoning');
    const summary = kept.getByTestId('thread-reasoning-summary');
    await expect(summary).toHaveText('Reasoning');
    expect(await isOpen(kept)).toBe(false);
    const thought = kept.getByText(THINKING.join(''));
    await expect(thought).toBeHidden();
    await summary.click();
    await expect(thought).toBeVisible();
    await expect(working).toHaveCount(0);
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('the working state is still when the person asks for less motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await fakeApi(page);
  await thinkingTurn(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const composer = byTestId(page, 'thread-composer-input');
  await composer.fill(THINKING_PROMPT);
  await composer.press('Enter');
  const label = byTestId(page, 'thread-working-label');
  await expect(label).toBeVisible();
  const moving = await byTestId(page, 'thread-working').evaluate(
    (working) => working.getAnimations({ subtree: true }).length,
  );
  expect(moving).toBe(0);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect
    .poll(() =>
      byTestId(page, 'thread-working').evaluate(
        (working) => working.getAnimations({ subtree: true }).length,
      ),
    )
    .toBe(2);
});
