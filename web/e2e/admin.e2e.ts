import { fakeAdmin, INVITE_CODE } from './fake-admin';
import { fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`An Owner opens Admin from their menu, changes a role, invites someone and removes someone (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    const admin = await fakeAdmin(page);
    await page.goto(`/${SIGNED_IN}`);
    await byTestId(page, 'person').click();
    await byTestId(page, 'nav-admin').click();
    await expect(page).toHaveURL(/\/admin$/u);
    const people = byTestId(page, 'people-person');
    await expect(people).toHaveCount(3);
    await expect(people.first()).toContainText('Owner · you');
    await expect(byTestId(page, 'people-count')).toHaveText('3 people · 1 invite waiting');
    await expect(byTestId(page, 'people-waiting')).toContainText('invited by Sam Ortiz');
    await expectAccessible(page);

    // Priya becomes an Admin.
    await people.nth(2).getByTestId('people-role').click();
    await page.getByRole('menuitemradio', { name: 'Admin' }).click();
    await expect
      .poll(() => admin.sent)
      .toContainEqual({
        method: 'role/set',
        params: { person: '01900000-0000-7000-8000-0000000000d3', role: 'admin' },
      });

    // Alex is invited by email, as a Member.
    await byTestId(page, 'people-invite').click();
    await byTestId(page, 'invite-email').fill('alex@example.com');
    await expectAccessible(page);
    await byTestId(page, 'invite-make').click();
    await expect(byTestId(page, 'invite-link')).toHaveValue(
      `https://glave.example/#invite=${INVITE_CODE}`,
    );
    await expect(byTestId(page, 'invite-told')).toContainText('We emailed it to alex@example.com');
    await expectAccessible(page);
    await byTestId(page, 'invite-done').click();
    await expect(byTestId(page, 'people-waiting')).toHaveCount(2);

    // Sam is taken out, once Dana says so.
    await people.nth(1).getByTestId('people-remove').click();
    await expect(byTestId(page, 'people-remove-confirm')).toContainText('Remove Sam Ortiz?');
    await byTestId(page, 'people-remove-confirm-ok').click();
    await expect(people).toHaveCount(2);

    // The first invite is cancelled.
    await byTestId(page, 'people-cancel-invite').first().click();
    await expect(byTestId(page, 'people-waiting')).toHaveCount(1);
    await byTestId(page, 'admin-back').click();
    await expect(page).toHaveURL(/\/$/u);
  });
}

test('A person on their own server has no Admin, and Admin sends them to their chats', async ({
  page,
}) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'nav-settings')).toBeVisible();
  await expect(byTestId(page, 'nav-admin')).toHaveCount(0);
  await page.goto('/admin');
  await expect(page).not.toHaveURL(/\/admin/u);
});
