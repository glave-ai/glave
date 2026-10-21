import { readFileSync, readdirSync } from 'node:fs';

import { expect, test } from '@playwright/test';
import axe, { type Result } from 'axe-core';

import { fakeApi } from './fake-api';
import { SIGNED_IN, WCAG, byTestId, described, expectAccessible } from './support';

/** What axe found the moment the side nav's ⋯ appeared, and whether it was bound by then. */
interface Appeared {
  readonly violations: readonly Result[];
  /** Whether the ⋯ had its test hook, a binding, when axe was done. */
  readonly bound: boolean;
}

/** The page, with axe in it and what it found as the ⋯ appeared. */
type Watched = Window & { axe: typeof axe; appeared?: Promise<Appeared> };

/**
 * The file of the build glave serves that holds the side nav's ⋯ menu, which the side nav
 * loads once the page is idle (`@defer (on idle)`): the one that defines its component.
 */
function menuChunk(): string {
  const built = new URL('../dist/app/browser/', import.meta.url);
  const found = readdirSync(built).find(
    (file) =>
      file.endsWith('.js') &&
      /selectors:\[\[["'`]app-nav-list-menu["'`]\]\]/u.test(
        readFileSync(new URL(file, built), 'utf8'),
      ),
  );
  if (found === undefined) {
    throw new Error('the build has no chunk for the side nav’s menu: npm run build first');
  }
  return found;
}

test('the side nav’s ⋯, loaded late, is never in the page unnamed: axe passes the moment it appears', async ({
  page,
}) => {
  await fakeApi(page);
  // Its code stays on its way, as on a busy computer, until the test lets it come.
  let asked: () => void = () => undefined;
  const requested = new Promise<void>((resolve) => {
    asked = resolve;
  });
  let release: () => void = () => undefined;
  const released = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(`**/${menuChunk()}`, async (route) => {
    asked();
    await released;
    await route.fallback();
  });
  await page.goto(`/${SIGNED_IN}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  await requested;

  // Angular puts a deferred block in the page as soon as its code is in, and binds it (names,
  // test hooks) at its next change detection, the page's next task. So axe runs as the block
  // appears, in the page, before any task can: its own waits between rules, which would let one
  // in, are made microtasks until it is done. The ⋯ is found by its element, since its test
  // hook is a binding too.
  await page.evaluate(axe.source);
  await page.evaluate((tags) => {
    const watched = window as unknown as Watched;
    watched.appeared = new Promise<Appeared>((resolve, reject) => {
      const observer = new MutationObserver(() => {
        const menu = document.querySelector('app-nav-list-menu button');
        if (menu === null) {
          return;
        }
        observer.disconnect();
        const later = window.setTimeout;
        const soon = (handler: TimerHandler, timeout?: number, ...args: unknown[]): number => {
          if (typeof handler === 'function' && (timeout ?? 0) <= 1) {
            queueMicrotask(() => {
              Reflect.apply(handler, window, args);
            });
            return 0;
          }
          return later(handler, timeout, ...args);
        };
        window.setTimeout = soon as typeof window.setTimeout;
        watched.axe
          .run(document, { runOnly: { type: 'tag', values: [...tags] }, preload: false })
          .finally(() => {
            window.setTimeout = later;
          })
          .then((results) => {
            resolve({ violations: results.violations, bound: menu.hasAttribute('data-testid') });
          }, reject);
      });
      observer.observe(document.body, { childList: true, subtree: true });
    });
  }, WCAG);
  release();
  const appeared = await page.evaluate(() => (window as unknown as Watched).appeared);
  // Axe looked before the block was bound, and found nothing wrong.
  expect(appeared?.bound).toBe(false);
  expect(described(appeared?.violations ?? [])).toEqual([]);

  // Then it is bound: named, and reached.
  const more = byTestId(page, 'nav-project-more-trigger');
  await expect(more).toHaveAccessibleName('Options for glave');
  await expect(more).not.toHaveAttribute('inert');
  await expectAccessible(page);
});
