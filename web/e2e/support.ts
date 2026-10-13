import AxeBuilder from '@axe-core/playwright';
import { type Locator, type Page, expect } from '@playwright/test';

/** Where the app is served, and Glaze's showcase (playwright.config.ts). */
export const APP_URL = 'http://127.0.0.1:4310';
export const SHOWCASE_URL = 'http://127.0.0.1:4311';
/** Where a glave with nothing set up serves the app, for the first run (first-run.e2e.ts). */
export const FIRST_RUN_URL = 'http://127.0.0.1:4312';
/** The variable that names that glave's home, so the test can read its sign-in token. */
export const FIRST_RUN_HOME = 'GLAVE_E2E_FIRST_RUN_HOME';

/** The fragment of the link that signs the app in (`glave serve` prints one). */
export const SIGNED_IN = '#token=glave_local_test';

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

/** Fails on any WCAG 2.2 A or AA violation axe finds, colour contrast included. */
export async function expectAccessible(page: Page): Promise<void> {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  const found = results.violations.map(
    (violation) =>
      `${violation.id}: ${violation.help} (${violation.nodes.map((node) => node.target.join(' ')).join(', ')})`,
  );
  expect(found).toEqual([]);
}
