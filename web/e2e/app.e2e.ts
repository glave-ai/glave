import { expect, test } from '@playwright/test';

import { NEW_THREAD, PERSONAL, THREAD, WAITING, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, shownOpacity, watchForProblems } from './support';

test('without a sign-in, the app says how to get one, in plain words', async ({ page }) => {
  await fakeApi(page);
  await page.goto('/');
  await expect(page).toHaveURL(/\/sign-in$/u);
  await expect(byTestId(page, 'sign-in')).toContainText(
    'Open glave on this computer to sign this browser in',
  );
  await expect(byTestId(page, 'sign-in')).not.toContainText('serve');
  await expectAccessible(page);
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the link signs in, leaves the address bar clean and lists the threads (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    await page.goto(`/${SIGNED_IN}`);
    const project = byTestId(page, 'nav-project').filter({ hasText: 'glave' });
    await expect(project.getByTestId('nav-thread')).toContainText(['Review the release notes']);
    expect(page.url()).not.toContain('code');
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('a thread shows its history and streams a reply to a new message', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-steps-summary')).toContainText('4 steps');
  await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
  await expectAccessible(page);
  const reply = byTestId(page, 'thread-composer-input');
  await reply.fill('Thanks');
  await reply.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Glad to help.');
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a turn that worked shows its results first, every box across the column (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const reply = byTestId(page, 'thread-message').last();
    const changes = byTestId(page, 'turn-changes');
    const steps = byTestId(page, 'thread-steps');
    const summary = byTestId(page, 'thread-steps-summary');
    await expect(reply).toContainText('They pass now.');
    // The bar: the project, and the button of the thread's panel, which says where it runs.
    await expect(byTestId(page, 'thread-meta')).toHaveText('glave');
    await expect(byTestId(page, 'thread-panel')).toHaveAttribute('aria-pressed', 'true');
    await expect(byTestId(page, 'panel-where-part').first()).toHaveText(
      /^(Your Mac|This computer)$/u,
    );
    // What changed, then the receipt: how long it took, what it cost, and its steps.
    await expect(byTestId(page, 'turn-changes-count')).toHaveText('2 files');
    await expect(byTestId(page, 'turn-changes-file')).toHaveCount(2);
    await expect(byTestId(page, 'turn-changes-file').first()).toContainText('src/parse.rs');
    await expect(summary).toContainText('2m 05s');
    await expect(summary).toContainText('$0.0042');
    await expect(summary).toContainText('4 steps');
    const boxes = await Promise.all(
      [reply, changes, steps, byTestId(page, 'thread-composer')].map((each) => each.boundingBox()),
    );
    const [replyBox, changesBox, stepsBox] = boxes;
    expect((replyBox?.y ?? 0) < (changesBox?.y ?? 0)).toBe(true);
    expect((changesBox?.y ?? 0) < (stepsBox?.y ?? 0)).toBe(true);
    // Every box spans the column: the same edges, the same width.
    expect(new Set(boxes.map((box) => Math.round(box?.x ?? -1))).size).toBe(1);
    expect(new Set(boxes.map((box) => Math.round(box?.width ?? -1))).size).toBe(1);
    // The receipt is a disclosure: the keyboard opens it, and each call says its step in the
    // server's line.
    const receipts = byTestId(page, 'thread-receipt');
    await expect(receipts.first()).toBeHidden();
    await summary.focus();
    await page.keyboard.press('Enter');
    await expect(receipts).toHaveCount(4);
    await expect(receipts.first()).toBeVisible();
    await expect(receipts.nth(0)).toContainText('bash: cargo test');
    await expect(receipts.nth(1)).toContainText('edit src/parse.rs');
    await expect(receipts.nth(2)).toContainText('write tests/empty.rs (4 lines)');
    await expectAccessible(page);
    await page.keyboard.press('Enter');
    await expect(receipts.first()).toBeHidden();
    // A file opens its changes in full.
    await byTestId(page, 'turn-changes-file').first().click();
    await expect(byTestId(page, 'changes')).toBeVisible();
    await expect(byTestId(page, 'file')).toHaveCount(1);
    await expect(byTestId(page, 'file-change')).toContainText('fn parse');
    await page.keyboard.press('Escape');
    await expect(byTestId(page, 'changes')).toBeHidden();
  });

  test(`what waits for the person is a card after the receipt, answered in place (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    const api = await fakeApi(page);
    await page.goto(`/t/${WAITING}${SIGNED_IN}`);
    const card = byTestId(page, 'thread-approval');
    await expect(card).toContainText('Needs you');
    await expect(card).toContainText('Run this command?');
    await expect(card.locator('code')).toHaveText('git push -u origin docs/release-notes');
    // Why it asks is a sentence of its own, never part of the command.
    await expect(byTestId(page, 'thread-approval-caution')).toHaveText('It pushes to a remote.');
    await expect(card.getByRole('button')).toHaveText([
      'Allow once',
      'Allow for this thread',
      'Deny',
    ]);
    await expect(byTestId(page, 'thread-steps-summary')).toContainText('waiting');
    const steps = await byTestId(page, 'thread-steps').boundingBox();
    const box = await card.boundingBox();
    expect((steps?.y ?? Infinity) < (box?.y ?? 0)).toBe(true);
    expect(Math.round(box?.width ?? 0)).toBe(Math.round(steps?.width ?? -1));
    await expectAccessible(page);
    await byTestId(page, 'thread-approval-allow-once').click();
    await expect
      .poll(() => api.requests.find((request) => request.method === 'approval/decide')?.params)
      .toMatchObject({ approval: 'call_2', decision: 'allow_once' });
  });

  test(`a reply has a copy button, shown on hover and keyboard focus, that copies its text (${colorScheme})`, async ({
    page,
    context,
    browserName,
  }) => {
    test.skip(browserName === 'webkit', "Playwright's WebKit cannot be granted the clipboard");
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const messages = byTestId(page, 'thread-message');
    await expect(messages.last()).toContainText('They pass now.');
    // Only the reply: what the person wrote needs no copying.
    await expect(byTestId(page, 'message-copy')).toHaveCount(1);
    const copy = messages.last().getByTestId('message-copy');
    // Hidden until the pointer is over the message, or keyboard focus is in it.
    await page.mouse.move(0, 0);
    await expect.poll(() => shownOpacity(copy)).toBe(0);
    await copy.focus();
    await expect.poll(() => shownOpacity(copy)).toBe(1);
    await expectAccessible(page);
    await copy.press('Enter');
    await expect(copy).toHaveAttribute('aria-label', 'Copied');
    await expect(page.locator('[aria-live]', { hasText: 'Copied' })).toHaveCount(1);
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toContain('They pass now.');
    await messages.last().hover();
    await expect.poll(() => shownOpacity(copy)).toBe(1);
  });
}

test.describe('on a touch screen', () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test('the bar’s buttons and the composer’s are a finger’s size, the composer one row', async ({
    page,
  }) => {
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
    for (const testId of ['page-menu', 'thread-panel', 'thread-composer-options']) {
      const box = await byTestId(page, testId).boundingBox();
      expect([Math.round(box?.width ?? 0), Math.round(box?.height ?? 0)]).toEqual([44, 44]);
    }
    await expect(byTestId(page, 'thread-panel')).toHaveAccessibleName(
      'Show or hide the thread panel',
    );
    // One row: options, the text and send, side by side.
    const [options, input, send] = await Promise.all(
      ['thread-composer-options', 'thread-composer-input', 'thread-composer-send'].map((each) =>
        byTestId(page, each).boundingBox(),
      ),
    );
    expect((options?.x ?? 0) < (input?.x ?? 0)).toBe(true);
    expect((input?.x ?? 0) < (send?.x ?? 0)).toBe(true);
    expect(Math.round(options?.y ?? 0)).toBe(Math.round(send?.y ?? -1));
    // What the agent may do and the model wait behind the options button, in a sheet.
    const status = byTestId(page, 'thread-composer-status');
    await expect(status).toBeHidden();
    await byTestId(page, 'thread-composer-options').click();
    await expect(status).toBeVisible();
    await expect(status.getByTestId('permissions-trigger')).toBeVisible();
    await expect(status.getByTestId('model-picker-trigger')).toContainText('Claude Haiku 5.5');
    await expectAccessible(page);
    await page.keyboard.press('Escape');
    await expect(status).toBeHidden();
  });

  test('what waits for the person takes the whole width, one answer above another', async ({
    page,
  }) => {
    await fakeApi(page);
    await page.goto(`/t/${WAITING}${SIGNED_IN}`);
    const buttons = byTestId(page, 'thread-approval').getByRole('button');
    await expect(buttons).toHaveCount(3);
    const boxes = await Promise.all([0, 1, 2].map((index) => buttons.nth(index).boundingBox()));
    expect(new Set(boxes.map((box) => Math.round(box?.x ?? -1))).size).toBe(1);
    expect(new Set(boxes.map((box) => Math.round(box?.width ?? -1))).size).toBe(1);
    expect(boxes.every((box) => (box?.height ?? 0) >= 44)).toBe(true);
    expect((boxes[0]?.y ?? 0) < (boxes[1]?.y ?? 0)).toBe(true);
    await expectAccessible(page);
  });

  test('the copy button always shows, since nothing hovers', async ({ page }) => {
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const copy = byTestId(page, 'thread-message').last().getByTestId('message-copy');
    await expect(copy).toBeAttached();
    await expect.poll(() => shownOpacity(copy)).toBe(1);
  });
});

test('a new chat starts a thread and sends the first message', async ({ page }) => {
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  // Signed in with a browser session: glave runs on this computer, and says what stays here.
  await expect(byTestId(page, 'home-private')).toHaveText(
    'Private on this computer. Only the model you pick sees what you send.',
  );
  await expect(byTestId(page, 'home-where-trigger')).toContainText('A chat');
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Plan the launch');
  await prompt.press('Enter');
  await expect(page).toHaveURL(new RegExp(`/t/${NEW_THREAD}$`, 'u'));
  await expect(byTestId(page, 'thread-message').first()).toContainText('Plan the launch');
  expect(api.calls).toContain('thread/start');
});

test('New chat starts its thread in Chats, not in the project listed first', async ({ page }) => {
  const api = await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await byTestId(page, 'nav-new-chat').click();
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Plan the launch');
  await prompt.press('Enter');
  await expect(page).toHaveURL(new RegExp(`/t/${NEW_THREAD}$`, 'u'));
  const start = api.requests.find((request) => request.method === 'thread/start');
  expect(start?.params['project']).toBe(PERSONAL.id);
  const chats = byTestId(page, 'nav-chats');
  await expect(chats.getByTestId('nav-thread-title').first()).toHaveText('New chat');
  const project = byTestId(page, 'nav-project').filter({ hasText: 'glave' });
  await expect(project.getByTestId('nav-thread')).toHaveCount(4);
});

test('tasks, settings and memory open from the side nav', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  // A task is going, so Tasks shows, with how many.
  await expect(byTestId(page, 'nav-tasks')).toHaveAccessibleName('Tasks 1 going');
  await byTestId(page, 'nav-tasks').click();
  await expect(page.getByRole('heading', { name: 'Tasks' })).toBeVisible();
  await expectAccessible(page);
  // Settings is the person's: their row opens it, beside Sign out.
  await byTestId(page, 'person').click();
  await expect(byTestId(page, 'person-menu')).toBeVisible();
  await expect(byTestId(page, 'person-sign-out')).toBeVisible();
  await expectAccessible(page);
  await byTestId(page, 'nav-settings').click();
  await expect(byTestId(page, 'person-menu')).toBeHidden();
  await expect(byTestId(page, 'settings-server')).toContainText('glave 0.0.0');
  await expect(byTestId(page, 'settings-account')).toContainText('OpenRouter');
  await expectAccessible(page);
  // Memory lives in Settings.
  await byTestId(page, 'settings-memory').click();
  await expect(page.getByRole('heading', { name: 'Memory' })).toBeVisible();
  await expectAccessible(page);
});
