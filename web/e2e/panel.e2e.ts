import { fakeApi } from './fake-api';
import { DINNER, TESTS, VERBOSE } from './fake-panel';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a coding thread shows its facts beside it, each section with something (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.clock.setFixedTime(new Date(Date.parse(TESTS.started_at ?? '') + 42_000));
    await fakeApi(page, { panel: true });
    await page.goto(`/t/${VERBOSE}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-message').last()).toContainText('the tests are running');

    // Shown at first, beside the conversation, its button pressed, on Overview.
    await expect(byTestId(page, 'panel-panel')).toBeVisible();
    await expect(byTestId(page, 'thread-panel')).toHaveAttribute('aria-pressed', 'true');
    const tabs = byTestId(page, 'panel-tabs-tab');
    await expect(tabs).toHaveText(['Overview', 'Changes']);
    await expect(tabs.first()).toHaveAttribute('aria-selected', 'true');
    await expect(byTestId(page, 'panel-where-part')).toHaveText([
      /^(Your Mac|This computer)$/u,
      'glave · /work/glave',
      'glave/add-verbose · a separate copy',
    ]);
    await expect(byTestId(page, 'panel-plan')).toContainText('Plan: 1 of 4 steps done');
    await expect(byTestId(page, 'panel-plan-step')).toHaveCount(4);
    // The plan being built is the panel's to show: the conversation has no card of it.
    await expect(byTestId(page, 'thread-plan')).toHaveCount(0);
    await expect(byTestId(page, 'panel-context')).toHaveText('39% used');
    await expect(byTestId(page, 'panel-sign-in')).toHaveText('1 needs signing in again');
    await expect(byTestId(page, 'panel-changes-count')).toHaveText('2 files');
    await expect(byTestId(page, 'panel-changes')).toContainText('10 lines added, 4 removed');
    await expect(byTestId(page, 'command-text')).toHaveText('cargo test');
    await expect(byTestId(page, 'command-time')).toHaveText('42s');
    await expect(byTestId(page, 'command-stop')).toHaveAccessibleName('Stop cargo test');
    await expectAccessible(page);
    // Compact now asks the server to compact the thread's context.
    const compacted = page.waitForRequest(
      (request) => request.headers()['glave-method'] === 'thread/compact',
    );
    await byTestId(page, 'panel-compact').click();
    expect((await compacted).postDataJSON()).toMatchObject({ params: { thread: VERBOSE } });
    await expect(byTestId(page, 'panel-compact')).toHaveText('Compact now');

    // Changes shows the thread's files in full, in the panel itself.
    await tabs.filter({ hasText: 'Changes' }).click();
    const shown = byTestId(page, 'panel-tabs-panel');
    await expect(shown.getByTestId('file')).toHaveCount(2);
    await expect(shown.getByTestId('file-change').first()).toContainText('pub verbose: bool');
    await expectAccessible(page);
    await tabs.filter({ hasText: 'Overview' }).click();

    // Stop on the last command by keyboard: the keyboard's place goes to the panel's heading.
    await byTestId(page, 'command-stop').focus();
    await page.keyboard.press('Enter');
    await expect(byTestId(page, 'panel-running')).toHaveCount(0);
    await expect(byTestId(page, 'panel-heading')).toBeFocused();

    // Hidden from the header, and still hidden after a reload; the conversation's card shows
    // the plan meanwhile.
    await byTestId(page, 'thread-panel').click();
    await expect(byTestId(page, 'panel-panel')).toBeHidden();
    await expect(byTestId(page, 'thread-plan')).toContainText('Plan: 1 of 4 steps done');
    await page.reload();
    await expect(byTestId(page, 'thread-message').last()).toBeVisible();
    await expect(byTestId(page, 'thread-panel')).toHaveAttribute('aria-pressed', 'false');
    await expect(byTestId(page, 'panel-panel')).toBeHidden();
    await byTestId(page, 'thread-panel').click();
    await expect(byTestId(page, 'panel-panel')).toBeVisible();
    await expect(byTestId(page, 'thread-plan')).toHaveCount(0);
    expect(problems()).toEqual([]);
  });
}

test('a plain chat shows no panel, no edge and no button for one', async ({ page }) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await page.goto(`/t/${DINNER}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').last()).toContainText('taco bar');
  await expect(byTestId(page, 'panel-panel')).toBeHidden();
  await expect(byTestId(page, 'panel-handle')).toHaveCount(0);
  await expect(byTestId(page, 'thread-panel')).toHaveCount(0);
  await expect(byTestId(page, 'thread-meta')).toHaveCount(0);
  // Its shortcut has nothing to show.
  await byTestId(page, 'thread-composer-input').focus();
  await page.keyboard.press('ControlOrMeta+.');
  await expect(byTestId(page, 'panel-panel')).toBeHidden();
  await expectAccessible(page);
  expect(problems()).toEqual([]);
});
