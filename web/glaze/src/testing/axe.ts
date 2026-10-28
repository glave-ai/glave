import axe, { type ElementContext } from 'axe-core';

/**
 * Fails when axe finds a WCAG A or AA violation in `context`: an element, or one with the parts
 * another test checks left out. Colour contrast needs a real browser, so it is checked end to
 * end (e2e/), not here.
 */
export async function expectAccessible(context: ElementContext): Promise<void> {
  const results = await axe.run(context, {
    runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] },
    rules: { 'color-contrast': { enabled: false } },
  });
  const found = results.violations.map(
    (violation) =>
      `${violation.id}: ${violation.help} (${violation.nodes.map((node) => node.html).join(', ')})`,
  );
  expect(found).toEqual([]);
}
