import AxeBuilder from '@axe-core/playwright';
import { type Page, expect } from '@playwright/test';

/** Elements end-to-end tests use, by their `data-testid` (AGENTS.md, "Test hooks"). */
export function byTestId(page: Page, testId: string): ReturnType<Page['getByTestId']> {
  return page.getByTestId(testId);
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
