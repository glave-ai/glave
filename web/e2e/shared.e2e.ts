import { oneDevice } from './devices';
import {
  SHARED,
  SHARED_WEBKIT,
  byTestId,
  expect,
  expectAccessible,
  served,
  sharedOrigin,
  signInCode,
  test,
  watchForProblems,
} from './support';

/**
 * People who share one computer, against a real glave whose origin is set to its
 * `http://localhost:<port>` (`specs/session.md`, "Doors"), and the real app: the person on the
 * computer, signed in at its address, alone, invites someone; a second browser opens the
 * link at `localhost`, makes a passkey and joins; the owner, now shown Admin, gives them a
 * role, cancels a second invite and takes them out, and the browser of the person taken out
 * is signed out. Each engine has a glave of its own, since the test fills it with people.
 */
test('on a shared computer, the owner invites someone, who joins, and is given a role and taken out', async ({
  browser,
  browserName,
  page,
}) => {
  const site = browserName === 'webkit' ? SHARED_WEBKIT : SHARED;
  const problems = watchForProblems(page);
  await page.goto(`${served(site)}/#code=${await signInCode(site)}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();

  // Alone, the owner invites someone from their menu.
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'nav-admin')).toHaveCount(0);
  await byTestId(page, 'nav-invite').click();
  await byTestId(page, 'invite-make').click();
  const link = await byTestId(page, 'invite-link').inputValue();
  expect(link.startsWith(`${sharedOrigin(site)}/#invite=`)).toBe(true);
  await byTestId(page, 'invite-done').click();

  // In a browser of their own, the person invited makes a passkey and joins.
  const theirs = await browser.newContext();
  const invited = await oneDevice(theirs, browserName).tab();
  await invited.goto(link);
  await expect(byTestId(invited, 'invite-passkey')).toBeVisible();
  await expectAccessible(invited);
  await byTestId(invited, 'invite-make-passkey').click();
  await expect(byTestId(invited, 'home-composer-input')).toBeVisible();

  // Two people now: the owner's menu leads to Admin, where they give the newcomer a role.
  await page.reload();
  await byTestId(page, 'person').click();
  await byTestId(page, 'nav-admin').click();
  const people = byTestId(page, 'people-person');
  await expect(people).toHaveCount(2);
  const newcomer = people.filter({ has: byTestId(page, 'people-remove') });
  await newcomer.getByTestId('people-role').click();
  await page.getByRole('menuitemradio', { name: 'Admin' }).click();
  await expect(newcomer.getByTestId('people-role')).toContainText('Admin');
  await expectAccessible(page);

  // A second invite, cancelled.
  await byTestId(page, 'people-invite').click();
  await byTestId(page, 'invite-make').click();
  await byTestId(page, 'invite-done').click();
  await expect(byTestId(page, 'people-waiting')).toHaveCount(1);
  await byTestId(page, 'people-cancel-invite').click();
  await expect(byTestId(page, 'people-waiting')).toHaveCount(0);

  // Taken out: their browser is signed out at its next request.
  await newcomer.getByTestId('people-remove').click();
  await byTestId(page, 'people-remove-confirm-ok').click();
  await expect(people).toHaveCount(1);
  await invited.reload();
  await expect(invited).toHaveURL(/\/sign-in$/u);
  await theirs.close();
  expect(problems()).toEqual([]);
});
