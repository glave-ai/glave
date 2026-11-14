import { fakeApi } from './fake-api';
import { personal } from './fake-personal';
import { SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

test('A person in their personal space creates an organization to invite people, and works in it', async ({
  page,
}) => {
  await fakeApi(page);
  const asked = await personal(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'nav-invite')).toHaveCount(0);
  await expect(byTestId(page, 'nav-create-to-invite')).toHaveText(
    'Create an organization to invite people',
  );
  await expectAccessible(page);
  await byTestId(page, 'nav-new-organization').click();
  await expect(byTestId(page, 'new-organization')).toBeVisible();
  await expectAccessible(page);
  await byTestId(page, 'new-organization-name').fill('Nurfz');
  await byTestId(page, 'new-organization-create').click();

  // The page starts again in it: headed with its name, inviting, and switching back.
  await expect(byTestId(page, 'nav-heading')).toHaveText('Nurfz');
  expect(asked.created).toEqual([{ name: 'Nurfz' }]);
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'nav-invite')).toBeVisible();
  await expect(byTestId(page, 'nav-create-to-invite')).toHaveCount(0);
  await expect(byTestId(page, 'person-switch')).toHaveCount(2);
});
