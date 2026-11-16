import { LONG } from './fake-long';
import { fakeApi } from './fake-api';
import { SIGNED_IN, atRest, byTestId, expect, test, watchForProblems } from './support';

/** A window short enough that a few turns fill the conversation. */
const SHORT = { width: 1280, height: 560 };

/** The `seq` of the long thread's turn `turn`'s end (from 1): 8 events each, from `seq` 3. */
function endOf(turn: number): number {
  return 3 + (turn - 1) * 8 + 7;
}

test('a tapped notification of a chat’s end opens it at the line where that turn finished, which stays in view', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  await page.setViewportSize(SHORT);
  await fakeApi(page);
  // Ten turns before the latest, so a conversation stuck to its end would show other turns.
  await page.goto(`/t/${LONG}?finished=${String(endOf(1240))}${SIGNED_IN}`);
  const away = byTestId(page, 'turn-away');
  await expect(away).toHaveCount(1);
  await expect(away).toContainText('Finished while you were away');
  await expect(away).toBeInViewport();
  await atRest(page);
  // Once the page has settled, the line and the reply under it still show, and the latest
  // turn does not: the conversation let go of its end, as "Jump to latest" says.
  await expect(away).toBeInViewport();
  await expect(page.getByText('Step 1240 is done.', { exact: true })).toBeInViewport();
  await expect(page.getByText('Step 1250 is done.', { exact: true })).not.toBeInViewport();
  await expect(byTestId(page, 'thread-latest')).toBeVisible();
  expect(problems()).toEqual([]);
});
