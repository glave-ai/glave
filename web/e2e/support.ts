import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import AxeBuilder from '@axe-core/playwright';
import { type Locator, type Page, expect } from '@playwright/test';

/** Where the app is served, and Glaze's showcase (playwright.config.ts). */
export const APP_URL = 'http://127.0.0.1:4310';
export const SHOWCASE_URL = 'http://127.0.0.1:4311';
/** Where a glave with nothing set up serves the app, for the first run (first-run.e2e.ts). */
export const FIRST_RUN_URL = 'http://127.0.0.1:4312';
/** The variable that names that glave's home, so the test can read its local token. */
export const FIRST_RUN_HOME = 'GLAVE_E2E_FIRST_RUN_HOME';
/** Where a glave with a model serves the app, for connecting accounts (accounts.e2e.ts). */
export const ACCOUNTS_URL = 'http://127.0.0.1:4313';
/** The variable that names that glave's home. */
export const ACCOUNTS_HOME = 'GLAVE_E2E_ACCOUNTS_HOME';

/** The fragment of the link that signs the app in, with a code the fake API takes. */
export const SIGNED_IN = '#code=e2e-code';

/**
 * A sign-in link's one-time code from the real glave at `url` whose home is `home`, asked for
 * with its local token, as `glave app` asks for one.
 */
export async function signInCode(url: string, home: string): Promise<string> {
  const token = readFileSync(join(home, 'serve.token'), 'utf8').trim();
  const response = await fetch(`${url}/`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'signin/link', params: {} }),
  });
  const answer = (await response.json()) as { readonly result?: { readonly code?: unknown } };
  const code = answer.result?.code;
  if (typeof code !== 'string') {
    throw new Error(`glave gave no sign-in code: ${JSON.stringify(answer)}`);
  }
  return code;
}

/** Elements end-to-end tests use, by their `data-testid` (AGENTS.md, "Test hooks"). */
export function byTestId(page: Page, testId: string): ReturnType<Page['getByTestId']> {
  return page.getByTestId(testId);
}

/** How opaque an element shows: its own opacity times its ancestors'. */
export async function shownOpacity(locator: Locator): Promise<number> {
  return locator.evaluate((element) => {
    let opacity = 1;
    for (let node: Element | null = element; node !== null; node = node.parentElement) {
      opacity *= Number(getComputedStyle(node).opacity);
    }
    return opacity;
  });
}

/** A box on the screen, as Playwright measures it. */
interface Box {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/** How far a box may stick out of another, for sub-pixel rounding. */
const ROUNDING = 1;

/** Fails unless `inner` has a size and lies inside `outer`. */
function expectInside(name: string, inner: Box, outer: Box): void {
  const inside =
    inner.width > 0 &&
    inner.height > 0 &&
    inner.x >= outer.x - ROUNDING &&
    inner.y >= outer.y - ROUNDING &&
    inner.x + inner.width <= outer.x + outer.width + ROUNDING &&
    inner.y + inner.height <= outer.y + outer.height + ROUNDING;
  expect(inside, `${name} ${JSON.stringify(inner)} shows inside ${JSON.stringify(outer)}`).toBe(
    true,
  );
}

/** The box of the element with this `data-testid`; fails when it has none. */
async function boxOf(page: Page, testId: string): Promise<Box> {
  const element = byTestId(page, testId);
  await expect(element).toBeVisible();
  const box = await element.boundingBox();
  if (box === null) {
    throw new Error(`${testId} has no box`);
  }
  return box;
}

/**
 * Fails unless the dialog `testId` shows whole, measured by the boxes on the screen: the dialog
 * inside the viewport, and its close button (in its header), its body and each of `parts` (what
 * the body holds, the actions) inside the dialog, none of them empty. A dialog cut short hides
 * what overflows it, so a part outside its box is a part the person cannot see.
 */
export async function expectDialogShown(
  page: Page,
  testId: string,
  parts: readonly string[],
): Promise<void> {
  const size = page.viewportSize();
  if (size === null) {
    throw new Error('the page has no viewport');
  }
  const dialog = await boxOf(page, testId);
  expectInside(testId, dialog, { x: 0, y: 0, ...size });
  for (const part of [`${testId}-close`, `${testId}-body`, ...parts]) {
    expectInside(part, await boxOf(page, part), dialog);
  }
}

/** Collects what the page says went wrong: errors on the console, and CSP violations. */
export function watchForProblems(page: Page): () => readonly string[] {
  const problems: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') {
      problems.push(message.text());
    }
  });
  page.on('pageerror', (error) => problems.push(error.message));
  return () => problems;
}

/**
 * Fails on any WCAG 2.2 A or AA violation axe finds, colour contrast included. Colours are
 * measured once they stop changing: a button fades to its new background after a switch to
 * dark, and halfway through the fade its text would fail.
 */
export async function expectAccessible(page: Page): Promise<void> {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((animation) => animation instanceof CSSTransition)
        .map((transition) => transition.finished.catch(() => undefined)),
    ),
  );
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  const found = results.violations.map(
    (violation) =>
      `${violation.id}: ${violation.help} (${violation.nodes.map((node) => node.target.join(' ')).join(', ')})`,
  );
  expect(found).toEqual([]);
}
