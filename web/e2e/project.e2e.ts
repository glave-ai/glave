import { PROJECT, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

test('a project’s page, where glave run in its folder opens, starts a thread in it', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  const api = await fakeApi(page);
  // As glave opens it: the project's address, signed in by the link's code.
  await page.goto(`/p/${PROJECT.id}${SIGNED_IN}`);
  await expect(page).toHaveURL(new RegExp(`/p/${PROJECT.id}$`, 'u'));
  await expect(page).toHaveTitle('Project · glave');
  await expect(byTestId(page, 'home-where-trigger')).toContainText('glave');
  // The cursor waits in the composer for the project's next thread.
  await expect(byTestId(page, 'home-composer-input')).toBeFocused();
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  // Its folder is a git repository, so a chat may work on a separate copy of it.
  const separate = byTestId(page, 'home-separate-input');
  await expect(separate).not.toBeChecked();
  await expect(separate).toHaveAccessibleName('Work on a separate copy');
  await expect(separate).toHaveAccessibleDescription(
    "so it doesn't touch your folder or other chats",
  );
  await separate.check();
  await byTestId(page, 'home-composer-input').fill('What changed today?');
  await byTestId(page, 'home-composer-input').press('Enter');
  await expect(page).toHaveURL(/\/t\//u);
  const started = api.requests.filter(({ method }) => method === 'thread/start');
  expect(started.at(-1)?.params).toMatchObject({ project: PROJECT.id, worktree: true });
  expect(problems()).toEqual([]);
});

test('a chat in the Chats has no separate copy to offer', async ({ page }) => {
  const problems = watchForProblems(page);
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'home-where-trigger')).toContainText('A chat');
  await expect(byTestId(page, 'home-separate')).toHaveCount(0);
  await byTestId(page, 'home-composer-input').fill('Dinner ideas?');
  await byTestId(page, 'home-composer-input').press('Enter');
  await expect(page).toHaveURL(/\/t\//u);
  const started = api.requests.filter(({ method }) => method === 'thread/start');
  expect(started.at(-1)?.params).not.toHaveProperty('worktree');
  expect(problems()).toEqual([]);
});
