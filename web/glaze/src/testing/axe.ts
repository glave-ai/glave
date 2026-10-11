import axe from 'axe-core';

/**
 * Fails when axe finds a WCAG A or AA violation in `root`. Colour contrast needs a real
 * browser, so it is checked end to end (e2e/), not here.
 */
export async function expectAccessible(root: Element): Promise<void> {
  const results = await axe.run(root, {
    runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] },
    rules: { 'color-contrast': { enabled: false } },
  });
  const found = results.violations.map(
    (violation) =>
      `${violation.id}: ${violation.help} (${violation.nodes.map((node) => node.html).join(', ')})`,
  );
  expect(found).toEqual([]);
}
