import { RAN, SERVED, SERVES } from './fake-model';
import {
  LIVE,
  byTestId,
  expect,
  expectAccessible,
  served,
  signInCode,
  test,
  watchForProblems,
} from './support';

/**
 * A real glave, its model the fake service of fake-model.ts: a server the agent keeps running in
 * the background is previewed beside the chat, at an address of its own on this computer, in a
 * frame under glave's Content Security Policy; Open preview on its command shows it, and
 * stopping the command takes the Preview tab away.
 */
test('a page a command serves shows in the Preview tab, at an origin of its own', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${served(LIVE)}/#code=${await signInCode(LIVE)}`);
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Please serve a page');
  await prompt.press('Enter');
  await expect(byTestId(page, 'thread-approval')).toContainText(SERVES);
  await byTestId(page, 'thread-approval-allow-once').click();
  await expect(byTestId(page, 'thread-message').last()).toContainText(RAN);

  // The Preview tab comes once the server listens, which the panel looks for again a moment
  // after the command starts; Open preview on the command shows it.
  const tab = byTestId(page, 'panel-tabs-tab').filter({ hasText: 'Preview' });
  await expect(tab).toBeVisible({ timeout: 20_000 });
  await byTestId(page, 'command-preview').click();
  await expect(tab).toHaveAttribute('aria-selected', 'true');
  const address = byTestId(page, 'preview-address');
  await expect(address).toHaveText(/^http:\/\/127\.0\.0\.1:\d+\/$/u);
  const url = await address.textContent();
  expect(url).not.toContain(new URL(served(LIVE)).port);
  await expect(byTestId(page, 'preview-open')).toHaveAttribute('href', url ?? '');
  await expect(byTestId(page, 'preview-open')).toHaveAttribute('target', '_blank');
  const frame = page.frameLocator('[data-testid="preview-frame"]');
  await expect(frame.locator('h1')).toHaveText(SERVED);
  // It fills the tab, not a frame's default size.
  const box = await byTestId(page, 'preview-frame').boundingBox();
  expect(box?.height ?? 0).toBeGreaterThan(400);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  // Reload draws the page afresh.
  await byTestId(page, 'preview-reload').click();
  await expect(frame.locator('h1')).toHaveText(SERVED);

  // Stopped, its preview goes, and the tab with it.
  await byTestId(page, 'panel-tabs-tab').filter({ hasText: 'Overview' }).click();
  await byTestId(page, 'command-stop').click();
  await expect(byTestId(page, 'command')).toHaveCount(0);
  await expect(tab).toHaveCount(0);
  expect(problems()).toEqual([]);
});
