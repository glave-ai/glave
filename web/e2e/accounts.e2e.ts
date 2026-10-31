import type { Locator, Page } from '@playwright/test';
import { type IncomingMessage, type Server, type ServerResponse, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';

import { fakeAppearance } from './fake-appearance';
import { fakeApi } from './fake-api';
import { RESEARCH } from './fake-sources';
import {
  ADMIN_WORDS,
  ACCOUNTS,
  byTestId,
  expect,
  expectAccessible,
  SIGNED_IN,
  atRest,
  served,
  signInCode,
  test,
  watchForProblems,
} from './support';

/** The token the person pastes, and one the fake host refuses. */
const TOKEN = 'gta_e2e0connected0account0wxyz';
const WRONG = 'gta_e2e0wrong0token0abcd';

/**
 * Answers as a Gitea or Forgejo server's API does: whom a token is for at `/api/v1/user`,
 * refusing any other token, and nothing at GitLab's or GitHub Enterprise's paths.
 */
function serve(request: IncomingMessage, response: ServerResponse): void {
  if (request.url !== '/api/v1/user') {
    response.writeHead(404, { 'content-type': 'application/json' });
    response.end('{}');
    return;
  }
  if (request.headers.authorization !== `token ${TOKEN}`) {
    response.writeHead(401, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ message: 'token is required' }));
    return;
  }
  response.writeHead(200, { 'content-type': 'application/json' });
  response.end(JSON.stringify({ login: 'ana' }));
}

// One person on one real glave: an accent one test chooses is what the other draws, so one at a time.
test.describe.configure({ mode: 'serial' });

let host: Server;

test.beforeAll(async () => {
  host = createServer(serve);
  await new Promise<void>((resolve) => host.listen(0, '127.0.0.1', resolve));
});

test.afterAll(async () => {
  await new Promise((resolve) => host.close(resolve));
});

test('a git host is connected with its address and a token, which never shows', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  const glave = served(ACCOUNTS);
  const code = await signInCode(ACCOUNTS);
  const { port } = host.address() as AddressInfo;
  const address = `http://127.0.0.1:${String(port)}`;

  await page.goto(`${glave}/#code=${code}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  await page.goto(`${glave}/settings`);
  await expect(page.getByRole('heading', { name: 'Connected accounts' })).toHaveCount(0);
  await page.goto(`${glave}/settings/connections`);
  await expect(byTestId(page, 'github-connect')).toBeVisible();
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  await byTestId(page, 'another-address').fill(address);
  await byTestId(page, 'another-add').click();
  await expect(byTestId(page, 'another-asks')).toContainText(
    `127.0.0.1:${String(port)} needs an access token`,
  );
  await byTestId(page, 'another-token').fill(WRONG);
  await byTestId(page, 'another-connect').click();
  await expect(byTestId(page, 'another-problem')).toContainText('did not accept that token');
  await expectAccessible(page);

  await byTestId(page, 'another-token').fill(TOKEN);
  await byTestId(page, 'another-connect').click();
  const account = byTestId(page, 'git-host');
  await expect(account).toContainText(`127.0.0.1:${String(port)} (Gitea)`);
  await expect(account).toContainText('Signed in as ana');
  await expect(byTestId(page, 'another-token')).toHaveCount(0);

  await page.reload();
  await expect(byTestId(page, 'git-host')).toContainText('Signed in as ana');
  expect(await page.content()).not.toContain(TOKEN);
  expect(await page.locator('body').innerText()).not.toMatch(/token ending/u);
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);
  await page.goto(`${glave}/settings`);
  await expect(byTestId(page, 'settings-connected')).toContainText(
    `127.0.0.1:${String(port)} (Gitea) is connected.`,
  );

  await page.goto(`${glave}/settings/connections`);
  await byTestId(page, 'git-host-disconnect').click();
  await byTestId(page, 'github-disconnect-confirm-ok').click();
  await expect(byTestId(page, 'git-host')).toHaveCount(0);
  expect(problems()).toEqual([]);
});

test('a person’s accent is the server’s, drawn in two values on the root, under the real page’s rules', async ({
  page,
}) => {
  const glave = served(ACCOUNTS);
  const code = await signInCode(ACCOUNTS);
  await page.goto(`${glave}/#code=${code}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  await page.goto(`${glave}/settings`);
  const accent = (name: string): Promise<string> =>
    page.evaluate((token) => document.documentElement.style.getPropertyValue(token), name);
  // In the order offered: no accent, then the swatches, yellow last.
  const accents = byTestId(page, 'appearance-accent-option');
  const [plain, yellow] = [accents.first(), accents.last()];
  await expect(byTestId(page, 'appearance-line')).toContainText('plain look');
  await expectAccessible(page);
  await expect.poll(() => accent('--glz-accent-light')).toBe('');

  // Yellow can't be read on white as it is: tuned, light moves and dark stays near it.
  await yellow.click();
  await expect(byTestId(page, 'appearance-line')).toContainText('your own color');
  await expect.poll(() => accent('--glz-accent-light')).toMatch(/^#[0-9a-f]{6}$/u);
  expect(await accent('--glz-accent-light')).not.toBe('#f2c200');
  await expectAccessible(page);

  // The exact color, with the line saying what it risks, kept across a reload. It may fail
  // contrast on purpose: what is promised is that color, in both, and the warning.
  await byTestId(page, 'appearance-adjust-input').uncheck();
  await expect(byTestId(page, 'appearance-adjust-line')).toContainText('may be hard to read');
  await expect.poll(() => accent('--glz-accent-light')).toBe('#f2c200');
  await page.reload();
  await expect(yellow.locator('input')).toBeChecked();
  expect(await accent('--glz-accent-dark')).toBe('#f2c200');

  // Any exact color, through the browser's own picker.
  await byTestId(page, 'appearance-accent-any').fill('#123456');
  await expect.poll(() => accent('--glz-accent-light')).toBe('#123456');

  await plain.click();
  await expect.poll(() => accent('--glz-accent-light')).toBe('');
});

/**
 * The two values a real glave draws an accent in, tuned, chosen by the person on it through
 * `choose`, which leaves it plain again after.
 */
async function realShades(
  page: Page,
  choose: (accents: Locator) => Promise<void>,
): Promise<{ readonly light: string; readonly dark: string }> {
  const glave = served(ACCOUNTS);
  const code = await signInCode(ACCOUNTS);
  await page.goto(`${glave}/#code=${code}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  await page.goto(`${glave}/settings`);
  const accents = byTestId(page, 'appearance-accent-option');
  await expect(accents.first()).toBeVisible();
  const root = (token: string): Promise<string> =>
    page.evaluate((name) => document.documentElement.style.getPropertyValue(name), token);
  await choose(accents);
  await expect.poll(() => root('--glz-accent-light')).toMatch(/^#[0-9a-f]{6}$/u);
  const shades = { light: await root('--glz-accent-light'), dark: await root('--glz-accent-dark') };
  await accents.first().click();
  await expect.poll(() => root('--glz-accent-light')).toBe('');
  return shades;
}

for (const [hue, choose] of [
  ['yellow, which can’t be read as it is', (accents: Locator) => accents.last().click()],
  ['purple', (accents: Locator) => accents.nth(5).click()],
  [
    'a dark blue, through the picker',
    (accents: Locator) => accents.page().getByTestId('appearance-accent-any').fill('#123456'),
  ],
] as const) {
  test(`tuned, ${hue} reads at AA on a primary button and in a link, light and dark`, async ({
    page,
    context,
  }) => {
    const shades = await realShades(page, choose);
    // A thread with both, drawn in what that glave said.
    const thread = await context.newPage();
    await fakeApi(thread, { research: true });
    await fakeAppearance(thread, shades);
    await thread.goto(`/t/${RESEARCH}${SIGNED_IN}`);
    await expect(byTestId(thread, 'reply-sources-link')).toHaveCount(2);
    await byTestId(thread, 'thread-composer-input').fill('Thanks');
    const send = byTestId(thread, 'thread-composer-send');
    for (const colorScheme of ['light', 'dark'] as const) {
      await thread.emulateMedia({ colorScheme });
      await atRest(thread);
      // The color settled, so nothing measures one on its way.
      await thread.waitForFunction(() =>
        document.getAnimations().every((each) => !(each instanceof CSSTransition)),
      );
      const fill = await send.evaluate((button) => getComputedStyle(button).backgroundColor);
      const seen = await thread.evaluate((color) => {
        const probe = document.createElement('span');
        probe.style.color = color;
        document.body.append(probe);
        const drawn = getComputedStyle(probe).color;
        probe.remove();
        return drawn;
      }, shades[colorScheme]);
      expect(fill, 'the button is in the accent, not the default').toBe(seen);
      await expectAccessible(thread);
    }
    await thread.close();
  });
}
