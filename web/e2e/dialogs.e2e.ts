import { expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectDialogShown } from './support';

/** A dialog the app has: where it opens from, the button that opens it, and what must show. */
interface Dialog {
  readonly name: string;
  readonly from: string;
  /** Whether the person has a git host connected, for Settings to list. */
  readonly account?: boolean;
  readonly opener: string;
  readonly dialog: string;
  /** The body's field, when it has one, and every action. */
  readonly shows: readonly string[];
}

/**
 * Every dialog the app has. Each opens in Chromium and in WebKit, so a dialog cut short in
 * Safari fails here first.
 */
const DIALOGS: readonly Dialog[] = [
  {
    name: 'Rename the thread',
    from: `/t/${THREAD}`,
    opener: 'thread-rename',
    dialog: 'rename',
    shows: ['rename-input', 'rename-cancel', 'rename-save'],
  },
  {
    name: 'New project',
    from: '/',
    opener: 'nav-new-project',
    dialog: 'new-project',
    shows: ['new-project-name', 'new-project-cancel', 'new-project-create'],
  },
  {
    name: 'Removing a model account',
    from: '/settings',
    opener: 'settings-account-remove',
    dialog: 'settings-remove',
    shows: ['settings-remove-cancel', 'settings-remove-ok'],
  },
  {
    name: 'Removing a connected account',
    from: '/settings',
    account: true,
    opener: 'accounts-remove',
    dialog: 'accounts-remove-confirm',
    shows: ['accounts-remove-confirm-cancel', 'accounts-remove-confirm-ok'],
  },
  {
    name: 'Editing a memory',
    from: '/memory',
    opener: 'memory-edit',
    dialog: 'memory-editor',
    shows: ['memory-text', 'memory-cancel', 'memory-save'],
  },
  {
    name: 'Forgetting a memory',
    from: '/memory',
    opener: 'memory-forget',
    dialog: 'memory-forget-confirm',
    shows: ['memory-forget-chat', 'memory-forget-confirm-cancel', 'memory-forget-confirm-ok'],
  },
];

for (const { name, from, account = false, opener, dialog, shows } of DIALOGS) {
  test(`${name} shows its body and its buttons, on the screen`, async ({ page }) => {
    await fakeApi(page, { account });
    await page.goto(`${from}${SIGNED_IN}`);
    await byTestId(page, opener).click();
    await expectDialogShown(page, dialog, shows);
    // The browser moves focus in, to its first control, the close button, named: in a dialog
    // made as it opens (New project's), too, whose buttons are out of reach until bound.
    await expect(byTestId(page, `${dialog}-close`)).toBeFocused();
  });
}

test('on a phone, a dialog shows its body and its buttons too', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await byTestId(page, 'thread-rename').click();
  await expectDialogShown(page, 'rename', ['rename-input', 'rename-cancel', 'rename-save']);
});
