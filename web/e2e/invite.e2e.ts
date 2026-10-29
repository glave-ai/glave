import { fakeApi } from './fake-api';
import { OPENS, SPENT, fakeInvite } from './fake-invite';
import { SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`An invite's link asks someone new to make a passkey and join (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    await fakeInvite(page);
    await page.goto(`/#invite=${OPENS}`);
    await expect(page).toHaveURL(/\/invite$/u);
    const asked = byTestId(page, 'invite-passkey');
    await expect(asked).toContainText('Dana Lee invited you to glave');
    await expect(asked).toContainText("You'll join as a member.");
    await expect(byTestId(page, 'invite-make-passkey')).toBeVisible();
    await expectAccessible(page);
  });

  test(`Someone signed in joins the team an invite is to, as one more of theirs (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    const invite = await fakeInvite(page);
    await page.goto(`/${SIGNED_IN}`);
    await expect(byTestId(page, 'home-composer-input')).toBeVisible();
    // Another page's address, so the tab loads it anew with the invite, as opening a link does.
    await page.goto(`/tasks#invite=${OPENS}`);
    await expect(byTestId(page, 'invite-joining')).toContainText('Join this team?');
    await expectAccessible(page);
    await byTestId(page, 'invite-join').click();
    await expect(byTestId(page, 'home-composer-input')).toBeVisible();
    expect(invite.accepted).toEqual([{ code: OPENS }]);
  });
}

test('An invite that opens nothing says so, and to ask for a new one', async ({ page }) => {
  await fakeApi(page);
  await fakeInvite(page);
  await page.goto(`/#invite=${SPENT}`);
  await expect(byTestId(page, 'invite-gone')).toContainText("This invite doesn't work anymore");
  await expectAccessible(page);
  await byTestId(page, 'invite-open').click();
  await expect(page).not.toHaveURL(/\/invite/u);
});
