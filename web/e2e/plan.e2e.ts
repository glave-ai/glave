import { expect, test } from '@playwright/test';

import { fakeApi } from './fake-api';
import { letGo } from './fake-gate';
import { PLANNING, planServer } from './fake-plan';
import { SIGNED_IN, byTestId, expectAccessible, hold, watchForProblems } from './support';

/** The steps the person edits the proposal to. */
const EDITED = [
  'Add --verbose and -v to Args, printing each step it takes',
  'Run cargo test',
  'Say what --verbose does in the README',
];

test('a proposed plan is edited, then built in Auto, its steps ticking off', async ({ page }) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  const { requests } = await fakeApi(page, { plan: true });
  await planServer(page);
  await page.goto(`/t/${PLANNING}${SIGNED_IN}`);
  const card = byTestId(page, 'thread-plan');
  await expect(card).toContainText('Proposed plan: 3 steps');
  await expect(card).toContainText('Needs you');
  await expect(byTestId(page, 'permissions-trigger')).toContainText('Plan');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  // The person edits it: one step changed, the first taken out, one added.
  await byTestId(page, 'thread-plan-edit').click();
  const fields = byTestId(page, 'thread-plan-input');
  await expect(fields).toHaveCount(3);
  await fields.nth(1).fill(EDITED[0] ?? '');
  await byTestId(page, 'thread-plan-remove').first().click();
  await byTestId(page, 'thread-plan-add').click();
  await expect(fields).toHaveCount(3);
  await fields.last().fill(EDITED[2] ?? '');
  await expectAccessible(page);
  const saving = await hold(page, '/', 'plan/edit');
  await byTestId(page, 'thread-plan-save').click();
  await saving.sent;
  // Until the server keeps it, the plan's buttons wait.
  await expect(byTestId(page, 'thread-plan-build')).toBeDisabled();
  saving.letGo();
  const steps = byTestId(page, 'thread-plan-step');
  await expect(steps).toHaveText(EDITED);
  await expect(byTestId(page, 'thread-plan-build')).toBeEnabled();
  expect(requests.filter((request) => request.method === 'plan/edit')).toEqual([
    { method: 'plan/edit', params: { thread: PLANNING, steps: EDITED } },
  ]);

  // Build it: the thread runs in Auto, and the plan ticks off as the model keeps it.
  await byTestId(page, 'thread-plan-build').click();
  await expect(card).toContainText('Plan: 1 of 3 steps done');
  await expect(card).not.toContainText('Needs you');
  await expect(byTestId(page, 'thread-plan-build')).toHaveCount(0);
  const marks = steps.locator('glz-icon');
  await expect(marks.nth(0)).toHaveAttribute('aria-label', 'Done');
  await expect(marks.nth(1)).toHaveAttribute('aria-label', 'Doing');
  await expect(marks.nth(2)).toHaveAttribute('aria-label', 'To do');
  await expect(byTestId(page, 'permissions-trigger')).toContainText('Auto');
  await expect(byTestId(page, 'thread-message').filter({ hasText: 'Build it' })).toHaveCount(1);
  await expect(page.getByText('The person approved this plan')).toHaveCount(0);
  await expectAccessible(page);
  await letGo(page, 'step');
  await expect(card).toContainText('Plan: 3 of 3 steps done');
  await expect(marks.nth(2)).toHaveAttribute('aria-label', 'Done');
  await expect(byTestId(page, 'thread-message').last()).toContainText('the tests pass');
  expect(requests.filter((request) => request.method === 'plan/build')).toEqual([
    { method: 'plan/build', params: { thread: PLANNING } },
  ]);
  await expectAccessible(page);
  expect(problems()).toEqual([]);
});
