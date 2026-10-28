import { fakeApi } from './fake-api';
import { DINNER, TESTS, VERBOSE } from './fake-panel';
import {
  SIGNED_IN,
  byTestId,
  expect,
  expectAccessible,
  expectDialogShown,
  test,
  watchForProblems,
} from './support';

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

    // Shown at first, beside the conversation, its button pressed.
    await expect(byTestId(page, 'panel-panel')).toBeVisible();
    await expect(byTestId(page, 'thread-panel')).toHaveAttribute('aria-pressed', 'true');
    await expect(byTestId(page, 'panel-where-part')).toHaveText([
      /^(Your Mac|This computer)$/u,
      'glave',
      'Auto',
    ]);
    await expect(byTestId(page, 'panel-plan')).toContainText('Plan: 1 of 4 steps done');
    await expect(byTestId(page, 'panel-plan-step')).toHaveCount(4);
    // The plan being built is the panel's to show: the conversation has no card of it.
    await expect(byTestId(page, 'thread-plan')).toHaveCount(0);
    // Both the count and each file's lines show at the panel's width.
    await expect(byTestId(page, 'panel-changes-count')).toBeVisible();
    await expect(byTestId(page, 'panel-changes-count')).toHaveText('2 files');
    await expect(byTestId(page, 'panel-changes-file').first().locator('.counts')).toBeVisible();
    await expect(byTestId(page, 'panel-changes-file')).toHaveCount(2);
    await expect(byTestId(page, 'command-text')).toHaveText('cargo test');
    await expect(byTestId(page, 'command-time')).toHaveText('42s');
    await expect(byTestId(page, 'command-stop')).toHaveAccessibleName('Stop cargo test');
    await expect(byTestId(page, 'panel-page')).toContainText('clap: Arg::action');
    await expect(byTestId(page, 'panel-page')).toHaveAttribute('target', '_blank');
    await expect(byTestId(page, 'panel-attached')).toContainText('terminal-output.png');
    await expect(byTestId(page, 'panel-context')).toHaveText('39% used');
    await expect(byTestId(page, 'panel-cost')).toHaveText('$0.01 · 2 turns');
    await expectAccessible(page);
    // The folder's full path is a key or a tap away.
    const path = byTestId(page, 'panel-path');
    await expect(path).toBeHidden();
    await byTestId(page, 'panel-folder').locator('summary').focus();
    await page.keyboard.press('Enter');
    await expect(path).toHaveText('/work/glave');

    // A file opens its changes in full over the page.
    await byTestId(page, 'panel-changes-file').first().click();
    await expect(byTestId(page, 'changes')).toBeVisible();
    await expect(byTestId(page, 'file')).toHaveCount(1);
    await expect(byTestId(page, 'file-change')).toContainText('pub verbose: bool');
    await expectDialogShown(page, 'changes', ['file']);
    await expectAccessible(page);
    await page.keyboard.press('Escape');
    await expect(byTestId(page, 'changes')).toBeHidden();

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
