import { type Locator, type Page, expect, test } from '@playwright/test';

import { MARKDOWN_PROMPT, MARKDOWN_REPLY, THREAD, fakeApi, streamReply } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, watchForProblems } from './support';

/** A window short enough that the thread, with its markdown reply, scrolls. */
const SHORT = { width: 1280, height: 560 };

/** Opens the thread and sends the prompt its next turn streams a long markdown reply to. */
async function askForMarkdown(page: Page): Promise<void> {
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
  const composer = byTestId(page, 'thread-composer-input');
  await composer.fill(MARKDOWN_PROMPT);
  await composer.press('Enter');
}

/** Where `locator` shows on the screen; fails when it has no box. */
async function boxOf(locator: Locator): Promise<{
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}> {
  const box = await locator.boundingBox();
  if (box === null) {
    throw new Error('it has no box');
  }
  return box;
}

/** How far the conversation is from its end, in pixels. */
async function fromEnd(page: Page): Promise<number> {
  return byTestId(page, 'thread-items').evaluate(
    (items) => items.scrollHeight - items.scrollTop - items.clientHeight,
  );
}

/** Whether the element's bottom edge shows inside the conversation's scrolled view. */
async function bottomShows(page: Page, testId: string): Promise<boolean> {
  return page.evaluate((testId) => {
    const items = document.querySelector('[data-testid="thread-items"]');
    const shown = Array.from(document.querySelectorAll(`[data-testid="${testId}"]`)).at(-1);
    if (items === null || shown === undefined) {
      return false;
    }
    const view = items.getBoundingClientRect();
    const bottom = shown.getBoundingClientRect().bottom;
    return bottom > view.top && bottom <= view.bottom + 1;
  }, testId);
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`"Jump to latest" floats over the conversation once scrolled up, and takes the person back (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize(SHORT);
    await fakeApi(page);
    await streamReply(page, MARKDOWN_REPLY, 0);
    await askForMarkdown(page);
    await expect(byTestId(page, 'thread-message').last()).toContainText('Document the change');
    const latest = byTestId(page, 'thread-latest');
    await expect(latest).toBeHidden();
    // At the end, the conversation runs down to the composer: nothing sits between them.
    const items = byTestId(page, 'thread-items');
    const itemsBox = await boxOf(items);
    const composerBox = await boxOf(byTestId(page, 'thread-composer'));
    expect(Math.abs(itemsBox.y + itemsBox.height - composerBox.y)).toBeLessThanOrEqual(1);

    await items.evaluate((element) => element.scrollTo({ top: 0 }));
    await expect(latest).toBeVisible();
    await expectAccessible(page);
    // Laid over the conversation's bottom edge, which stays where it was, centred over the
    // composer and clear of it.
    expect(await boxOf(items)).toEqual(itemsBox);
    const box = await boxOf(latest);
    expect(box.y).toBeGreaterThan(itemsBox.y);
    expect(box.y + box.height).toBeLessThanOrEqual(itemsBox.y + itemsBox.height);
    expect(box.y + box.height).toBeLessThanOrEqual(composerBox.y);
    const middle = box.x + box.width / 2;
    expect(Math.abs(middle - (composerBox.x + composerBox.width / 2))).toBeLessThanOrEqual(1);

    await latest.getByRole('button', { name: 'Jump to latest' }).click();
    await expect(latest).toBeHidden();
    await expect.poll(() => fromEnd(page)).toBeLessThanOrEqual(1);
    await expect.poll(() => bottomShows(page, 'thread-message')).toBe(true);
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}

test('while the person is at the end, a streaming reply keeps its last line in view', async ({
  page,
}) => {
  await page.setViewportSize(SHORT);
  await fakeApi(page);
  await streamReply(page);
  await askForMarkdown(page);
  // Partway through, the line being written shows, and nothing offers to jump.
  await expect(byTestId(page, 'thread-reply').locator('th').first()).toBeVisible();
  expect(await bottomShows(page, 'thread-reply')).toBe(true);
  await expect(byTestId(page, 'thread-latest')).toBeHidden();
  // And once it ends, its last line still does.
  await expect(byTestId(page, 'thread-message').last()).toContainText('evil.example/pixel.png');
  await expect.poll(() => bottomShows(page, 'thread-message')).toBe(true);
  expect(await fromEnd(page)).toBeLessThanOrEqual(1);
  await expect(byTestId(page, 'thread-latest')).toBeHidden();
});
