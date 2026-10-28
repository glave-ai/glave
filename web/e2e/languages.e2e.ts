import { readFileSync } from 'node:fs';

import { type Page, expect, test } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, readIn, watchForProblems } from './support';

/**
 * The plain words the pages may show in `en-XA`: what the server wrote (the fixtures' titles,
 * names, model and reply), names of brands, and what `Intl` writes in a language it does not
 * know but English. Anything else plain is a word of the app's that skipped its messages.
 */
const NOT_THE_APPS = new Set([
  'Check the parser’s edge cases',
  'Claude Haiku 5.5',
  'Dinner ideas for Saturday',
  'Make the tests pass',
  // The fake server's line for a run of calls: the server writes it in the caller's language.
  'Ran 4 calls',
  'Nightly dependency audit',
  'Review the release notes',
  'Summarize open issues',
  'They pass now. parse() returned a value for empty input and unwrapped a failed parse; it now returns None for both, and a new test covers the empty case.',
  'src/parse.rs',
  'tests/empty.rs',
  'glave',
  'GitHub',
  'OpenRouter',
  '5 minutes ago',
  '6 minutes ago',
  '9 minutes ago',
]);

/**
 * The words of the app the page shows, or names for screen readers, left plain: with a word
 * of English and no accented letter, the mark of every `en-XA` message.
 */
async function skipped(page: Page): Promise<readonly string[]> {
  const plain = await page.evaluate(() => {
    const accented = /[åƀçðéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇÐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ]/u;
    const words = /[A-Za-z]{2,}/u;
    const found = new Set<string>();
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const text = node.textContent?.trim() ?? '';
      const parent = node.parentElement;
      const shown =
        parent !== null &&
        (parent.checkVisibility({ visibilityProperty: true }) ||
          parent.closest('.glz-visually-hidden') !== null);
      if (shown && words.test(text) && !accented.test(text)) {
        found.add(text);
      }
    }
    for (const element of document.querySelectorAll('[aria-label], [placeholder], [title]')) {
      for (const name of ['aria-label', 'placeholder', 'title']) {
        const text = element.getAttribute(name) ?? '';
        if (words.test(text) && !accented.test(text)) {
          found.add(text);
        }
      }
    }
    return [...found];
  });
  return plain.filter((text) => !NOT_THE_APPS.has(text)).sort();
}

/** `en-XA`'s messages, as `npm run i18n` made them, by id. */
const PSEUDO = (
  JSON.parse(
    readFileSync(new URL('../app/src/locale/messages.en-XA.json', import.meta.url), 'utf8'),
  ) as { readonly translations: Readonly<Record<string, string>> }
).translations;

/** Ten minutes after the fixtures' threads were last active, in UTC. */
const NOW = new Date('2026-10-11T10:10:00.000Z');
test.use({ timezoneId: 'UTC' });

test('a browser set to en-XA reads every word of the app in it, plurals too', async ({ page }) => {
  const problems = watchForProblems(page);
  await readIn(page, 'en-XA');
  await page.clock.setFixedTime(NOW);
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(page.locator('html')).toHaveAttribute('lang', 'en-XA');
  await expect(byTestId(page, 'nav-new-chat')).toHaveAccessibleName(
    PSEUDO['sideNav.newChat'] ?? '',
  );
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  expect(await skipped(page)).toEqual([]);
  await expectAccessible(page);

  await problems.leave(`/t/${THREAD}`);
  await expect(page).toHaveTitle(PSEUDO['routes.thread'] ?? '');
  await expect(byTestId(page, 'thread-steps-summary')).toContainText('4 šţéþš');
  await expect(byTestId(page, 'turn-changes-count')).toHaveText('[2 ƒîļéš one]');
  expect(await skipped(page)).toEqual([]);
  await expectAccessible(page);
  // The thread's panel beside it, and a dialog over it.
  await expect(byTestId(page, 'panel-changes').locator('h3')).toHaveText(
    PSEUDO['threadPanel.changes'] ?? '',
  );
  expect(await skipped(page)).toEqual([]);
  await byTestId(page, 'thread-rename').click();
  await expect(byTestId(page, 'rename')).toBeVisible();
  expect(await skipped(page)).toEqual([]);
  await expectAccessible(page);
  await byTestId(page, 'rename-cancel').click();

  await problems.leave('/settings');
  await expect(byTestId(page, 'settings-theme')).toBeVisible();
  expect(await skipped(page)).toEqual([]);
  await expectAccessible(page);
  expect(problems()).toEqual([]);
});

/** A box on the screen, as Playwright measures it. */
interface Box {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/** The box of the element with `testId`, which shows; fails when it has none. */
async function boxOf(page: Page, testId: string): Promise<Box> {
  const element = byTestId(page, testId);
  await expect(element).toBeVisible();
  const box = await element.boundingBox();
  if (box === null) {
    throw new Error(`${testId} has no box`);
  }
  return box;
}

/** Whether `one` lies wholly left of `other`, give or take a pixel of rounding. */
function leftOf(one: Box, other: Box): boolean {
  return one.x + one.width <= other.x + 1;
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a browser set to ar-XB reads the app right to left, the page mirrored (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await readIn(page, 'ar-XB');
    await page.clock.setFixedTime(NOW);
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    await expect(page.locator('html')).toHaveAttribute('dir', 'rtl');
    await expect(page.locator('html')).toHaveAttribute('lang', 'ar-XB');
    // The side nav on the right, the thread's panel on the left, the conversation and its
    // composer between them, nothing over anything else.
    const nav = await boxOf(page, 'workspace-nav');
    const tools = await boxOf(page, 'panel-panel');
    const conversation = await boxOf(page, 'thread-items');
    const composer = await boxOf(page, 'thread-composer');
    expect(leftOf(conversation, nav)).toBe(true);
    expect(leftOf(composer, nav)).toBe(true);
    expect(leftOf(tools, conversation)).toBe(true);
    expect(leftOf(tools, composer)).toBe(true);
    // A chevron that points to what comes next points left.
    const chevron = await byTestId(page, 'thread-steps-chevron').evaluate(
      (element) => getComputedStyle(element).scale,
    );
    expect(chevron).toBe('-1 1');
    await expectAccessible(page);
    // The side nav's edge faces left: the arrow pointing away from it widens it.
    const before = Math.round(nav.width);
    await byTestId(page, 'workspace-handle').focus();
    await page.keyboard.press('ArrowLeft');
    await expect
      .poll(async () => Math.round((await boxOf(page, 'workspace-nav')).width))
      .toBe(before + 16);
    // On a phone, the side nav is a drawer from the right, where its button is.
    await byTestId(page, 'panel-handle').press('Enter');
    await expect(byTestId(page, 'panel-panel')).toBeHidden();
    await page.setViewportSize({ width: 390, height: 800 });
    const menu = await boxOf(page, 'page-menu');
    await byTestId(page, 'page-menu').click();
    const drawer = await boxOf(page, 'workspace-nav');
    expect(Math.round(drawer.x + drawer.width)).toBe(390);
    expect(menu.x + menu.width / 2).toBeGreaterThan(390 / 2);
    await expectAccessible(page);
    expect(problems()).toEqual([]);
  });
}
