import { type Page } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, test } from './support';

/** How far what a composer's box holds sits from each inner edge of the box, in CSS pixels. */
interface Insets {
  readonly top: number;
  readonly end: number;
  readonly bottom: number;
  readonly start: number;
}

/** The space between the box of `composer` and what it shows, on each side. */
async function insets(page: Page, composer: string): Promise<Insets> {
  return byTestId(page, `${composer}-box`).evaluate((box) => {
    const style = getComputedStyle(box);
    const outer = box.getBoundingClientRect();
    const top = outer.top + parseFloat(style.borderTopWidth);
    const left = outer.left + parseFloat(style.borderLeftWidth);
    const right = outer.right - parseFloat(style.borderRightWidth);
    const bottom = outer.bottom - parseFloat(style.borderBottomWidth);
    const shown = Array.from(box.children, (child) => child.getBoundingClientRect()).filter(
      (rect) => rect.width > 0 && rect.height > 0,
    );
    const round = (value: number): number => Math.round(value * 10) / 10;
    const fromLeft = round(Math.min(...shown.map((rect) => rect.left)) - left);
    const fromRight = round(right - Math.max(...shown.map((rect) => rect.right)));
    const rtl = style.direction === 'rtl';
    return {
      top: round(Math.min(...shown.map((rect) => rect.top)) - top),
      end: rtl ? fromLeft : fromRight,
      bottom: round(bottom - Math.max(...shown.map((rect) => rect.bottom))),
      start: rtl ? fromRight : fromLeft,
    };
  });
}

/** Expects the four distances equal, and more than nothing. */
async function expectEven(page: Page, composer: string): Promise<void> {
  const found = await insets(page, composer);
  expect(found.top).toBeGreaterThan(0);
  expect(found).toEqual({ top: found.top, end: found.top, bottom: found.top, start: found.top });
}

/** The left and right of the element with `testId`. */
async function edges(page: Page, testId: string): Promise<readonly [number, number]> {
  const box = await byTestId(page, testId).boundingBox();
  expect(box, testId).not.toBeNull();
  return [Math.round(box?.x ?? 0), Math.round((box?.x ?? 0) + (box?.width ?? 0))];
}

for (const width of [1440, 390]) {
  test(`a composer's box is as far from what it holds on every side (${String(width)} wide)`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    await expect(byTestId(page, 'thread-composer-input')).toBeVisible();
    await expectEven(page, 'thread-composer');
    // With a file to send, on a row of its own above the text, the same.
    await byTestId(page, 'thread-composer-picker').setInputFiles([
      { name: 'notes.md', mimeType: 'text/markdown', buffer: Buffer.from('- [ ] ship it') },
    ]);
    await expect(byTestId(page, 'thread-composer-file')).toHaveCount(1);
    await expectEven(page, 'thread-composer');
    // Right to left, mirrored, the same.
    await page.evaluate(() => {
      document.documentElement.dir = 'rtl';
    });
    await expectEven(page, 'thread-composer');
    await page.goto(`/${SIGNED_IN}`);
    await expect(byTestId(page, 'home-composer-input')).toBeVisible();
    await expectEven(page, 'home-composer');
  });
}

test('the status line keeps the permissions at its start and the model at its end', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'model-picker-trigger')).toBeVisible();
  // Each end lines up with what the box holds above it: the + and send.
  const [start] = await edges(page, 'thread-composer-attach');
  const [, end] = await edges(page, 'thread-composer-send');
  const [permissions] = await edges(page, 'permissions-trigger');
  expect(permissions).toBe(start);
  // The default model reasons its own way: it alone is at the end.
  const [model, modelEnd] = await edges(page, 'model-picker-trigger');
  expect(modelEnd).toBe(end);
  expect(model).toBeGreaterThan((start + end) / 2);
  // A model with levels to choose: the level comes after it, the two together at the end.
  await byTestId(page, 'model-picker-trigger').click();
  await byTestId(page, 'model-picker-option').filter({ hasText: 'Hope' }).click();
  await expect(byTestId(page, 'effort-picker-trigger')).toBeVisible();
  const [hope, picked] = await edges(page, 'model-picker-trigger');
  const [effort, effortEnd] = await edges(page, 'effort-picker-trigger');
  expect(effortEnd).toBe(end);
  expect(effort - picked).toBeLessThan(16);
  expect(hope).toBeGreaterThan((start + end) / 2);
});
