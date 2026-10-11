/**
 * Test hooks for a component's parts: for each part, the `data-testid` `<testId>-<suffix>`, or
 * `null` when the component was given no `testId`, so nothing is added to the page. Glaze never
 * names hooks itself: the app chooses each `testId`, and end-to-end tests select by it.
 */
export function testIds<K extends string>(
  testId: string | null,
  parts: Readonly<Record<K, string>>,
): Readonly<Record<K, string | null>> {
  const entries = Object.entries<string>(parts).map(([key, suffix]) => [
    key,
    testId === null ? null : `${testId}-${suffix}`,
  ]);
  return Object.fromEntries(entries) as Record<K, string | null>;
}
