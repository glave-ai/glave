import { type Page, expect, test } from '@playwright/test';

import { MISSING_REPOSITORY, type Request, fakeApi } from './fake-api';
import { HOME_REFUSED } from './fake-folders';
import { SIGNED_IN_ELSEWHERE } from './fake-signin';
import { SIGNED_IN, byTestId, expectAccessible, shownOpacity, watchForProblems } from './support';

/** Words New project never says (AGENTS.md, "Keeping principle 4"). */
const NOT_SAID = /\bspaces?\b|workspace|policy|organization|member|admin|config\.toml|GLAVE_/iu;

/** The projects the fake makes, by id, in order. */
const FIRST = '01900000-0000-7000-8000-000000000900';

/** What `project/create` was asked. */
function created(requests: readonly Request[]): readonly Record<string, unknown>[] {
  return requests.filter(({ method }) => method === 'project/create').map(({ params }) => params);
}

/** Checks the page with axe in light and in dark. */
async function accessibleInBoth(page: Page): Promise<void> {
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
}

test('a project made from a folder picked on this computer, then its first thread', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  const add = byTestId(page, 'nav-new-project');
  await expect(add).toHaveAccessibleName('New project');
  // Like a project's own +, it shows while the pointer or keyboard focus is on the Projects
  // heading.
  await page.mouse.move(0, 0);
  await expect.poll(() => shownOpacity(add)).toBe(0);
  await add.focus();
  await expect.poll(() => shownOpacity(add)).toBe(1);
  await add.press('Enter');

  const dialog = byTestId(page, 'new-project');
  await expect(dialog).toBeVisible();
  await expect(page.getByRole('radio', { name: 'A folder on this computer' })).toBeChecked();
  // The picker opens at the home folder, which is no project's: it says so and can't be used.
  await expect(byTestId(page, 'folder-picker-crumb')).toHaveText(['Users', 'ana']);
  await expect(byTestId(page, 'folder-picker-folder')).toHaveText(['code', 'Documents']);
  await expect(byTestId(page, 'folder-picker-refused')).toHaveText(HOME_REFUSED);
  await expect(byTestId(page, 'folder-picker-use')).toBeDisabled();
  await expect(byTestId(page, 'new-project-create')).toBeDisabled();
  await accessibleInBoth(page);
  expect(await dialog.innerText()).not.toMatch(NOT_SAID);

  // Into code by the keyboard: focus follows into the folder.
  await byTestId(page, 'folder-picker-folder').first().press('Enter');
  await expect(byTestId(page, 'folder-picker-crumb')).toHaveText(['Users', 'ana', 'code']);
  await expect(byTestId(page, 'folder-picker-folder').first()).toBeFocused();
  await byTestId(page, 'folder-picker-folder').filter({ hasText: 'glave' }).click();
  await expect(byTestId(page, 'folder-picker-empty')).toBeVisible();
  await byTestId(page, 'folder-picker-use').click();
  await expect(byTestId(page, 'new-project-folder-path')).toHaveText('/Users/ana/code/glave');
  await expect(byTestId(page, 'new-project-name')).toHaveValue('glave');
  await accessibleInBoth(page);

  await byTestId(page, 'new-project-create').click();
  // The person already has a glave, so the server calls this one glave 2, as the app does.
  await expect(dialog).toBeHidden();
  await expect(page).toHaveURL(new RegExp(`/\\?project=${FIRST}$`, 'u'));
  // The cursor waits in the composer for the project's first message.
  await expect(byTestId(page, 'home-composer-input')).toBeFocused();
  await expect(byTestId(page, 'home-where-trigger')).toContainText('glave 2');
  // The project just made is the newest, so it comes first, though a thread in glave waits.
  await expect(byTestId(page, 'nav-project-toggle')).toHaveText(['glave 2', 'glave']);
  expect(created(api.requests)).toEqual([
    { name: 'glave', folder: { kind: 'path', path: '/Users/ana/code/glave' } },
  ]);

  // The first message starts the project's first thread.
  await byTestId(page, 'home-composer-input').fill('Read me the README');
  await byTestId(page, 'home-composer-input').press('Enter');
  await expect(page).toHaveURL(/\/t\//u);
  const started = api.requests.filter(({ method }) => method === 'thread/start');
  expect(started.at(-1)?.params['project']).toBe(FIRST);
  expect(problems()).toEqual([]);
});

test('a repository clones while it shows, and a refusal is told in the server’s words', async ({
  page,
}) => {
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'nav-new-project').click();
  await page.getByRole('radio', { name: 'A repository' }).check();
  await expect(byTestId(page, 'folder-picker-path')).toHaveCount(0);
  // No git host is connected: the line says where to connect one.
  await expect(byTestId(page, 'new-project-private')).toContainText(
    'Private repositories need a connected git host',
  );
  await expect(byTestId(page, 'new-project-connect')).toHaveAttribute('href', '/settings#accounts');
  // The server, not the browser, says what address it wants.
  await byTestId(page, 'new-project-url').fill('git@github.com:acme/site.git');
  await byTestId(page, 'new-project-create').click();
  await expect(byTestId(page, 'new-project-problem')).toHaveText(
    '"git@github.com:acme/site.git" is not a repository\'s address: give its https:// address, such as https://github.com/acme/app, with no user name or token in it.',
  );
  await byTestId(page, 'new-project-url').fill(MISSING_REPOSITORY);
  await expect(byTestId(page, 'new-project-name')).toHaveValue('missing');
  await byTestId(page, 'new-project-create').click();
  await expect(byTestId(page, 'new-project-cloning')).toBeVisible();
  await expect(byTestId(page, 'new-project-create')).toHaveText('Cloning…');
  await expectAccessible(page);
  await expect(byTestId(page, 'new-project-problem')).toHaveText(
    `Could not clone ${MISSING_REPOSITORY}: remote: Repository not found. If the repository is private, connect its git host first.`,
  );
  await expect(byTestId(page, 'new-project-cloning')).toBeHidden();
  await accessibleInBoth(page);

  await byTestId(page, 'new-project-url').fill('https://github.com/acme/site.git');
  await byTestId(page, 'new-project-url').press('Enter');
  await expect(byTestId(page, 'new-project')).toBeHidden();
  await expect(page).toHaveURL(/\?project=/u);
  await expect(byTestId(page, 'home-where-trigger')).toContainText('site');
  expect(created(api.requests)).toEqual([
    { name: 'site', folder: { kind: 'repository', url: 'git@github.com:acme/site.git' } },
    { name: 'missing', folder: { kind: 'repository', url: MISSING_REPOSITORY } },
    { name: 'site', folder: { kind: 'repository', url: 'https://github.com/acme/site.git' } },
  ]);
});

test('connecting a git host opens Settings at the connected accounts', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 480 });
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'nav-new-project').click();
  await page.getByRole('radio', { name: 'A repository' }).check();
  await byTestId(page, 'new-project-connect').click();
  await expect(byTestId(page, 'new-project')).toBeHidden();
  await expect(page).toHaveURL(/\/settings#accounts$/u);
  const accounts = byTestId(page, 'accounts');
  // Settings scrolls inside the main pane, which moved to the accounts, their heading clear of
  // the floating top bar.
  await expect
    .poll(() => accounts.evaluate((card) => card.closest('main')?.scrollTop ?? 0))
    .toBeGreaterThan(0);
  const heading = accounts.getByRole('heading', { name: 'Connected accounts' });
  await expect(heading).toBeInViewport();
  const uncovered = await heading.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const top = document.elementFromPoint(box.left + 4, box.top + box.height / 2);
    return top !== null && element.contains(top);
  });
  expect(uncovered).toBe(true);
});

test('an empty project needs a name, then glave makes its folder', async ({ page }) => {
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'nav-new-project').click();
  await page.getByRole('radio', { name: 'Empty' }).check();
  await expect(byTestId(page, 'new-project-create')).toBeDisabled();
  await byTestId(page, 'new-project-name').fill('Ideas');
  await byTestId(page, 'new-project-name').press('Enter');
  await expect(page).toHaveURL(new RegExp(`/\\?project=${FIRST}$`, 'u'));
  await expect(byTestId(page, 'home-where-trigger')).toContainText('Ideas');
  expect(created(api.requests)).toEqual([{ name: 'Ideas', folder: { kind: 'empty' } }]);
});

test('signed in from another computer, a project is made by its name alone', async ({ page }) => {
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN_ELSEWHERE}`);
  await byTestId(page, 'nav-new-project').click();
  await expect(byTestId(page, 'new-project')).toBeVisible();
  await expect(byTestId(page, 'new-project').getByRole('radio')).toHaveCount(0);
  await expect(byTestId(page, 'new-project-no-folder')).toHaveText(
    'Its threads start in an empty folder for now; a computer for it comes later.',
  );
  await accessibleInBoth(page);
  await byTestId(page, 'new-project-name').fill('Garden');
  await byTestId(page, 'new-project-create').click();
  await expect(byTestId(page, 'home-where-trigger')).toContainText('Garden');
  expect(api.calls).not.toContain('folder/list');
  expect(created(api.requests)).toEqual([{ name: 'Garden' }]);
});

test('Escape closes it and gives focus back to the +', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  const add = byTestId(page, 'nav-new-project');
  await add.focus();
  await add.press('Enter');
  await expect(byTestId(page, 'new-project')).toBeVisible();
  // Focus stays in it: the page behind is inert, so Tab never reaches it.
  for (let tab = 0; tab < 12; tab += 1) {
    await page.keyboard.press('Tab');
    const inside = await byTestId(page, 'new-project').evaluate(
      (dialog) =>
        dialog.contains(document.activeElement) || document.activeElement === document.body,
    );
    expect(inside).toBe(true);
  }
  await page.keyboard.press('Escape');
  await expect(byTestId(page, 'new-project')).toBeHidden();
  await expect(add).toBeFocused();
});

test.describe('on a touch screen', () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test('the + always shows, and Escape closes the dialog over the drawer, not the drawer', async ({
    page,
  }) => {
    await fakeApi(page);
    await page.goto(`/${SIGNED_IN}`);
    await byTestId(page, 'page-menu').click();
    const nav = byTestId(page, 'workspace-nav');
    await expect(nav).toBeVisible();
    const add = byTestId(page, 'nav-new-project');
    await expect.poll(() => shownOpacity(add)).toBe(1);
    await add.click();
    await expect(byTestId(page, 'new-project')).toBeVisible();
    await expectAccessible(page);
    await page.keyboard.press('Escape');
    await expect(byTestId(page, 'new-project')).toBeHidden();
    await expect(nav).toBeVisible();
  });

  test('made from the drawer, the drawer closes and the composer takes focus', async ({ page }) => {
    await fakeApi(page);
    await page.goto(`/${SIGNED_IN}`);
    await byTestId(page, 'page-menu').click();
    await byTestId(page, 'nav-new-project').click();
    await byTestId(page, 'folder-picker-folder').filter({ hasText: 'code' }).click();
    await expect(byTestId(page, 'folder-picker-crumb')).toHaveText(['Users', 'ana', 'code']);
    await byTestId(page, 'folder-picker-use').click();
    await expect(byTestId(page, 'new-project-name')).toHaveValue('code');
    await byTestId(page, 'new-project-create').click();
    await expect(byTestId(page, 'new-project')).toBeHidden();
    await expect(byTestId(page, 'workspace-nav')).toBeHidden();
    await expect(byTestId(page, 'home-composer-input')).toBeFocused();
  });
});
