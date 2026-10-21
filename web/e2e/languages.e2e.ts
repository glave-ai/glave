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
  'Nightly dependency audit',
  'Review the release notes',
  'They pass now. parse() returned a value for empty input and unwrapped a failed parse; it now returns None for both, and a new test covers the empty case.',
  'src/parse.rs',
  'tests/empty.rs',
  'glave',
  'GitHub',
  'OpenRouter',
  '5 minutes ago',
  '6 minutes ago',
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
  await expect(byTestId(page, 'nav-new-chat')).toHaveAccessibleName('[Ñéŵ çĥåţ one]');
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  expect(await skipped(page)).toEqual([]);
  await expectAccessible(page);

  await page.goto(`/t/${THREAD}`);
  await expect(byTestId(page, 'thread-steps-summary')).toContainText('4 šţéþš');
  await expect(byTestId(page, 'turn-changes-count')).toHaveText('[2 ƒîļéš one]');
  expect(await skipped(page)).toEqual([]);
  await expectAccessible(page);

  await page.goto('/settings');
  await expect(byTestId(page, 'settings-theme')).toBeVisible();
  expect(await skipped(page)).toEqual([]);
  await expectAccessible(page);
  expect(problems()).toEqual([]);
});
