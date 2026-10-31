import { fakeAdmin, INVITE_CODE } from './fake-admin';
import { fakeApi } from './fake-api';
import { LOGO_PNG, fakeBrand } from './fake-appearance';
import { ADMIN_WORDS, SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

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

test('A person alone on their own server invites someone from their menu, with no Admin', async ({
  page,
}) => {
  await fakeApi(page);
  const admin = await fakeAdmin(page, { alone: true });
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'nav-admin')).toHaveCount(0);
  await expect(byTestId(page, 'person-menu')).not.toContainText(ADMIN_WORDS);
  await byTestId(page, 'nav-invite').click();
  await expect(byTestId(page, 'invite')).toBeVisible();
  // Where glave sends no email, no address is asked for.
  await expect(byTestId(page, 'invite-email')).toHaveCount(0);
  await expectAccessible(page);
  await byTestId(page, 'invite-make').click();
  await expect(byTestId(page, 'invite-link')).toHaveValue(
    `https://glave.example/#invite=${INVITE_CODE}`,
  );
  expect(admin.sent).toContainEqual({ method: 'invite/create', params: { role: 'member' } });
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`An Owner gives the organization a name and an accent, seen in light and dark (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    await fakeAdmin(page);
    const brand = await fakeBrand(page);
    await page.goto(`/admin/brand${SIGNED_IN}`);
    await expect(byTestId(page, 'admin-brand')).toHaveAttribute('aria-current', 'page');
    await byTestId(page, 'brand-name').fill('Acme');
    // In the order offered: no accent, blue, lime, then a person's, teal first.
    await byTestId(page, 'brand-accent-option').nth(3).click();
    await expect(byTestId(page, 'brand-preview-light')).toBeVisible();
    await expect(byTestId(page, 'brand-preview-dark')).toBeVisible();
    await expectAccessible(page);

    await byTestId(page, 'brand-logo-file').setInputFiles({
      name: 'logo.png',
      mimeType: 'image/png',
      buffer: Buffer.from(LOGO_PNG, 'base64'),
    });
    await expect(byTestId(page, 'brand-logo')).toBeVisible();

    await byTestId(page, 'brand-adjust-input').uncheck();
    await expect(byTestId(page, 'brand-adjust-line')).toContainText('may be hard to read');
    await byTestId(page, 'brand-adjust-input').check();
    await byTestId(page, 'brand-locked-input').check();
    await expect(byTestId(page, 'brand-locked-line')).toContainText('Nobody');
    await byTestId(page, 'brand-save').click();
    await expect(byTestId(page, 'brand-saved')).toBeVisible();
    expect(brand.kept).toEqual([
      {
        name: 'Acme',
        logo: expect.any(String),
        accent: { color: { color: '#0f766e', adjust: true } },
        locked: true,
      },
    ]);

    // A person's side of it: the name in the corner, and no choice of accent left.
    await page.goto('/settings');
    // Drawn under the page's real rules: a logo that couldn't load has no width.
    const logo = byTestId(page, 'nav-logo');
    await expect(logo).toHaveAttribute('alt', 'Acme');
    await expect.poll(() => logo.evaluate((image: HTMLImageElement) => image.naturalWidth)).toBe(1);
    await expect(byTestId(page, 'appearance-line')).toContainText('Everyone in Acme');
    await expect(byTestId(page, 'appearance-accent')).toHaveCount(0);
  });
}
