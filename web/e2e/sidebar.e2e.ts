import { type Locator, type Page, expect, test } from '@playwright/test';

import { NEW_THREAD, PERSONAL, PROJECT, THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expectAccessible, shownOpacity } from './support';

/** The side nav's threads a person can see, in order. */
function shown(page: Page): Locator {
  return byTestId(page, 'nav-thread').filter({ visible: true });
}

/** The project's threads by recent activity: what waits first, a thread's own under it. */
const BY_ACTIVITY = [
  'Review the release notes needs you',
  'Nightly dependency audit running',
  'Make the tests pass',
  'Check the parser’s edge cases',
];

/** Opens the ⋯ of the list `testId` and picks how to sort it. */
async function sortBy(page: Page, testId: string, sort: string): Promise<void> {
  await byTestId(page, `${testId}-trigger`).first().click();
  await byTestId(page, `${testId}-choice`).filter({ hasText: sort }).click();
}

/** A row's marker as drawn: its width, and its contrast against the row's fill and the sidebar. */
interface Marker {
  readonly width: string;
  readonly againstFill: number;
  readonly againstSidebar: number;
}

/**
 * The bar beside the row `locator` (its `::before`), with its contrast as WCAG measures it
 * against the row's own background and the first one behind the row that is not transparent.
 */
async function markerOf(locator: Locator): Promise<Marker> {
  return locator.evaluate((element) => {
    // Any colour the browser computes (rgb, oklch, color()) as sRGB bytes, painted on a canvas.
    const canvas = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
    const channels = (color: string): readonly number[] => {
      canvas?.clearRect(0, 0, 1, 1);
      if (canvas !== null) {
        canvas.fillStyle = color;
        canvas.fillRect(0, 0, 1, 1);
      }
      return Array.from(canvas?.getImageData(0, 0, 1, 1).data ?? []);
    };
    const opaque = (color: string): boolean => (channels(color)[3] ?? 0) !== 0;
    const luminance = (color: string): number => {
      const [r = 0, g = 0, b = 0] = channels(color).map((value) => {
        const unit = value / 255;
        return unit <= 0.04045 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const contrast = (one: string, other: string): number => {
      const [a, b] = [luminance(one), luminance(other)];
      return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    };
    let sidebar = '';
    for (
      let node = element.parentElement;
      node !== null && sidebar === '';
      node = node.parentElement
    ) {
      const color = getComputedStyle(node).backgroundColor;
      sidebar = opaque(color) ? color : '';
    }
    const bar = getComputedStyle(element, '::before');
    return {
      width: bar.width,
      againstFill: contrast(bar.backgroundColor, getComputedStyle(element).backgroundColor),
      againstSidebar: contrast(bar.backgroundColor, sidebar),
    };
  });
}

test('projects come first, then Chats; each list by recent activity, what waits on top', async ({
  page,
}) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(shown(page)).toHaveText([...BY_ACTIVITY, 'Dinner ideas for Saturday']);
  const projects = await byTestId(page, 'nav-projects').boundingBox();
  const chats = await byTestId(page, 'nav-chats').boundingBox();
  expect(projects?.y ?? Infinity).toBeLessThan(chats?.y ?? 0);
  // A project's name heads its threads at their left edge; a thread another started is
  // indented under it.
  const heading = await byTestId(page, 'nav-space-toggle').boundingBox();
  const parent = byTestId(page, 'nav-thread').filter({ hasText: 'Make the tests pass' });
  const child = byTestId(page, 'nav-children').getByTestId('nav-thread');
  await expect(child).toHaveText('Check the parser’s edge cases');
  const [over, under] = [await parent.boundingBox(), await child.boundingBox()];
  expect(over?.x).toBe(heading?.x);
  expect(under?.x ?? 0).toBeGreaterThan(over?.x ?? Infinity);
  expect(under?.y ?? 0).toBeGreaterThan(over?.y ?? Infinity);
});

test('a thread moves up when it gets a new message', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(shown(page)).toHaveText([...BY_ACTIVITY, 'Dinner ideas for Saturday']);
  const reply = byTestId(page, 'thread-composer-input');
  await reply.fill('Thanks');
  await reply.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText('Glad to help.');
  await expect(shown(page)).toHaveText([
    'Review the release notes needs you',
    'Make the tests pass',
    'Check the parser’s edge cases',
    'Nightly dependency audit running',
    'Dinner ideas for Saturday',
  ]);
});

test('a list sorts by name from its ⋯, remembered in this browser', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await expect(shown(page)).toHaveText([...BY_ACTIVITY, 'Dinner ideas for Saturday']);
  await byTestId(page, 'nav-space-toggle').hover();
  await sortBy(page, 'nav-space-more', 'Name');
  const byName = [
    'Review the release notes needs you',
    'Make the tests pass',
    'Check the parser’s edge cases',
    'Nightly dependency audit running',
    'Dinner ideas for Saturday',
  ];
  await expect(shown(page)).toHaveText(byName);
  await page.reload();
  await expect(shown(page)).toHaveText(byName);
  await byTestId(page, 'nav-space-more-trigger').click();
  await expect(byTestId(page, 'nav-space-more-choice').filter({ hasText: 'Name' })).toHaveAttribute(
    'aria-checked',
    'true',
  );
  await page.keyboard.press('Escape');
  await expect(byTestId(page, 'nav-space-more-choice')).toHaveCount(0);
});

test('a heading opens and closes by click and by keyboard, remembered', async ({ page }) => {
  await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  const toggle = byTestId(page, 'nav-space-toggle');
  const caret = byTestId(page, 'nav-space-caret');
  await expect(toggle).toHaveText('glave');
  await page.mouse.move(0, 0);
  await expect.poll(() => shownOpacity(caret)).toBe(0);
  await toggle.hover();
  await expect.poll(() => shownOpacity(caret)).toBe(1);
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(shown(page)).toHaveText(['Dinner ideas for Saturday']);
  // Closed, glave still says a thread in it needs the person.
  await expect(byTestId(page, 'nav-space-end')).toHaveText('needs you');
  await expect(toggle).toHaveAccessibleDescription('needs you');
  await page.reload();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(shown(page)).toHaveText(['Dinner ideas for Saturday']);
  // By keyboard: Enter and Space toggle it, and the caret shows while it has focus.
  await page.mouse.move(0, 0);
  await toggle.focus();
  await expect.poll(() => shownOpacity(caret)).toBe(1);
  await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(byTestId(page, 'nav-space-end')).toHaveText('');
  await page.keyboard.press('Space');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await page.keyboard.press('Space');
  await expect(shown(page)).toHaveCount(5);
  // Projects and Chats close the same way.
  await byTestId(page, 'nav-projects-toggle').click();
  await byTestId(page, 'nav-chats-toggle').click();
  await expect(shown(page)).toHaveCount(0);
  await expect(byTestId(page, 'nav-projects-end')).toHaveText('needs you');
});

test('each + starts in its own place: a project, a thread in the project, a chat', async ({
  page,
}) => {
  const api = await fakeApi(page);
  await page.goto(`/${SIGNED_IN}`);
  await byTestId(page, 'nav-projects-toggle').hover();
  await byTestId(page, 'nav-new-project').click();
  await expect(byTestId(page, 'new-project')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(byTestId(page, 'new-project')).toBeHidden();

  await byTestId(page, 'nav-space-toggle').hover();
  await byTestId(page, 'nav-space-new').click();
  await expect(byTestId(page, 'home-where-trigger')).toContainText('glave');
  await byTestId(page, 'home-composer-input').fill('Plan the launch');
  await byTestId(page, 'home-composer-input').press('Enter');
  await expect(page).toHaveURL(new RegExp(`/t/${NEW_THREAD}$`, 'u'));

  await byTestId(page, 'nav-chats-toggle').hover();
  await byTestId(page, 'nav-chats-new').click();
  await expect(byTestId(page, 'home-where-trigger')).toContainText('A chat');
  await byTestId(page, 'home-composer-input').fill('Dinner for six');
  await byTestId(page, 'home-composer-input').press('Enter');
  await expect(page).toHaveURL(new RegExp(`/t/${NEW_THREAD}$`, 'u'));
  const started = api.requests.filter(({ method }) => method === 'thread/start');
  expect(started.map(({ params }) => params['space'])).toEqual([PROJECT.id, PERSONAL.id]);
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the thread shown is marked by a bar beside it, and the side nav passes axe (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await fakeApi(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const current = byTestId(page, 'nav-thread').filter({ hasText: 'Make the tests pass' });
    const other = byTestId(page, 'nav-thread').filter({ hasText: 'Nightly dependency audit' });
    await expect(current).toHaveAttribute('aria-current', 'page');
    await expect(other).not.toHaveAttribute('aria-current');
    const weight = (row: Locator): Promise<string> =>
      row.evaluate((element) => getComputedStyle(element).fontWeight);
    expect(await weight(current)).toBe(await weight(other));
    // A bar 3px wide, 3:1 or more against the row's fill and against the sidebar.
    const marker = await markerOf(current);
    expect(marker.width).toBe('3px');
    expect(marker.againstFill).toBeGreaterThanOrEqual(3);
    expect(marker.againstSidebar).toBeGreaterThanOrEqual(3);
    expect((await markerOf(other)).width).toBe('auto');
    await expectAccessible(page);
    // A closed project with its dot, and its menu open.
    await byTestId(page, 'nav-space-toggle').click();
    await expect(byTestId(page, 'nav-space-end')).toHaveText('needs you');
    await byTestId(page, 'nav-space-toggle').hover();
    await byTestId(page, 'nav-space-more-trigger').click();
    await expect(byTestId(page, 'nav-space-more-action')).toHaveText('Rename');
    await expectAccessible(page);
  });
}
