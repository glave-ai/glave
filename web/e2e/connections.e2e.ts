import { type ChildProcessByStdio, spawn } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import type { Readable, Writable } from 'node:stream';

import {
  ADMIN_WORDS,
  CONNECTIONS,
  CONNECTIONS_WEBKIT,
  TEST_SERVER,
  byTestId,
  expect,
  expectAccessible,
  hold,
  homeOf,
  served,
  signInCode,
  test,
  watchForProblems,
} from './support';

/** The token the service's sign-in grants, which it lets in. */
const TOKEN = 'e2e-connections';

/**
 * Words a person on their own computer should never meet (AGENTS.md, "Keeping principle 4"),
 * nor the name of the protocol or the file their services are set up in.
 */
const ADMIN_OR_FILES = new RegExp(`${ADMIN_WORDS.source}|\\bMCP\\b|mcp\\.json`, 'iu');

// Each engine's tests change one glave's settings, so they run one after another.
test.describe.configure({ mode: 'default' });

/** The service: glave's MCP test server, asking for a sign-in at its own fake. */
let service: ChildProcessByStdio<Writable, Readable, null>;
let address = '';

test.beforeAll(async () => {
  // It ends once its input closes: when this worker ends, however it ends.
  service = spawn(
    TEST_SERVER,
    ['--http', '--fake-sign-in', '--token', TOKEN, '--ends-with-stdin'],
    { stdio: ['pipe', 'pipe', 'inherit'] },
  );
  const lines = createInterface({ input: service.stdout });
  address = await new Promise<string>((resolve, reject) => {
    lines.once('line', resolve);
    service.once('error', reject);
    service.once('exit', (code) => {
      reject(new Error(`the test server ended first (${String(code)}): cargo xtask ci builds it`));
    });
  });
  lines.close();
});

test.afterAll(async () => {
  const ended = new Promise((resolve) => service.once('exit', resolve));
  service.stdin.end();
  await ended;
});

test('a service that asks for a sign-in is connected in a new tab, then disconnected', async ({
  page,
  context,
  browserName,
}) => {
  const problems = watchForProblems(page);
  const site = browserName === 'webkit' ? CONNECTIONS_WEBKIT : CONNECTIONS;
  const glave = served(site);
  const settings = { mcpServers: { Tracker: { url: address } } };
  writeFileSync(join(homeOf(site), 'mcp.json'), JSON.stringify(settings));
  const code = await signInCode(site);
  // One page load, signed in by the link where it lands. Loading another page would cut the
  // requests this one holds open, such as the side nav's watch, and WebKit says so on the console.
  await page.goto(`${glave}/settings#code=${code}`);
  await byTestId(page, 'settings-connections').click();
  await expect(page).toHaveURL(/\/settings\/connections$/u);
  await expect(page.getByRole('heading', { name: 'Connections', level: 1 })).toBeVisible();
  const available = byTestId(page, 'connections-available');
  await expect(available.getByTestId('connection')).toContainText('Tracker');
  await expect(byTestId(page, 'connections-ready')).toHaveCount(0);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  // The service's own sign-in waits until the test lets it go, so the row is seen waiting.
  const signIn = await hold(context, (url) => url.pathname === '/authorize');
  const opened = page.waitForEvent('popup');
  await available.getByTestId('connection-action').click();
  const tab = await opened;
  const waiting = byTestId(page, 'connection-waiting');
  await expect(waiting).toContainText('Finish signing in to Tracker in the new tab.');
  await expect(byTestId(page, 'connection-open')).toHaveAttribute('href', /\/authorize\?/u);
  await expect(available.getByTestId('connection-action')).toHaveText('Cancel');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  signIn.letGo();
  await expect(tab.locator('body')).toContainText('Signed in to Tracker.');
  const ready = byTestId(page, 'connections-ready');
  await expect(ready.getByTestId('connection')).toContainText('Connected');
  await expect(available).toHaveCount(0);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  await tab.close();
  expect(await page.content()).not.toContain(TOKEN);
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_OR_FILES);

  await ready.getByTestId('connection-action').click();
  await byTestId(page, 'connections-disconnect-ok').click();
  await expect(available.getByTestId('connection')).toContainText('Connect');
  await expect(ready).toHaveCount(0);
  expect(problems()).toEqual([]);
});

test('web search is a switch in Settings, and the composer shows a globe while it is on', async ({
  page,
  browserName,
}) => {
  const problems = watchForProblems(page);
  const site = browserName === 'webkit' ? CONNECTIONS_WEBKIT : CONNECTIONS;
  const glave = served(site);
  const settings = join(homeOf(site), 'mcp.json');
  writeFileSync(settings, JSON.stringify({ mcpServers: {} }));
  const code = await signInCode(site);
  await page.goto(`${glave}/settings#code=${code}`);
  const search = byTestId(page, 'web-search');
  await expect(search).toContainText('Uses Exa, free. Your searches go to Exa.');
  const box = byTestId(page, 'web-search-on-input');
  await expect(box).not.toBeChecked();
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  await box.click();
  await expect(box).toBeChecked();
  await expect(byTestId(page, 'web-search-usage')).toHaveText('No searches this month');
  const turnedOn = JSON.parse(readFileSync(settings, 'utf8')) as {
    readonly mcpServers: Readonly<Record<string, { readonly url?: string }>>;
  };
  expect(turnedOn.mcpServers['search']?.url).toMatch(/tools=web_search_exa$/u);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_OR_FILES);

  // It is no connection of the person's.
  await byTestId(page, 'settings-connections').click();
  await expect(page.getByRole('heading', { name: 'Connections', level: 1 })).toBeVisible();
  await expect(byTestId(page, 'github-row')).toBeVisible();
  await expect(byTestId(page, 'connections-ready')).toHaveCount(0);
  await byTestId(page, 'nav-new-chat').click();
  const globe = byTestId(page, 'home-searching');
  await expect(globe).toHaveAttribute('aria-label', 'Web search is on');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  await page.goBack();
  await page.goBack();
  // The switch shows the click at once; the file changes once the server answers it.
  const answered = page.waitForResponse(
    (response) => response.request().headers()['glave-method'] === 'search/set',
  );
  await byTestId(page, 'web-search-on-input').click();
  await expect(byTestId(page, 'web-search-on-input')).not.toBeChecked();
  await answered;
  expect(readFileSync(settings, 'utf8')).not.toContain('web_search_exa');
  await byTestId(page, 'nav-new-chat').click();
  await expect(byTestId(page, 'home-composer-box')).toBeVisible();
  await expect(globe).toHaveCount(0);
  expect(problems()).toEqual([]);
});

test('a second account of an app is added and named, each beneath it with its own menu', async ({
  page,
  browserName,
}) => {
  const problems = watchForProblems(page);
  const site = browserName === 'webkit' ? CONNECTIONS_WEBKIT : CONNECTIONS;
  const glave = served(site);
  writeFileSync(join(homeOf(site), 'mcp.json'), JSON.stringify({ mcpServers: {} }));
  const code = await signInCode(site);
  await page.goto(`${glave}/settings#code=${code}`);
  await byTestId(page, 'settings-connections').click();
  await expect(page.getByRole('heading', { name: 'Connections', level: 1 })).toBeVisible();

  // DeepWiki asks no one to sign in, so each account of it connects at once.
  const deepwiki = page.locator('[data-connector="deepwiki"]');
  await deepwiki.getByTestId('connector-action').click();
  const connected = byTestId(page, 'connectors-connected').locator('[data-connector="deepwiki"]');
  await expect(connected).toContainText('Everywhere, including chats');
  await connected.getByTestId('connector-add-account').click();
  const dialog = byTestId(page, 'connector-add');
  await expect(dialog).toContainText('Sign in to another DeepWiki account');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  await byTestId(page, 'connector-add-name').fill('Acme');
  await byTestId(page, 'connector-add-place').getByText('Only in Chats').click();
  await byTestId(page, 'connector-add-done').click();

  await expect(connected.getByTestId('connector-detail')).toContainText('2 accounts');
  const accounts = connected.getByTestId('connector-account');
  await expect(accounts).toHaveCount(2);
  await expect(accounts.nth(0)).toContainText('Everywhere, including chats');
  await expect(accounts.nth(1)).toContainText('Acme');
  await expect(accounts.nth(1)).toContainText('Only in Chats');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  // The first, never named, is named now.
  await accounts.nth(0).getByTestId('connector-account-menu-trigger').click();
  await byTestId(page, 'connector-account-menu-action').getByText('Rename').click();
  await byTestId(page, 'rename-input').fill('Glave');
  await byTestId(page, 'rename-save').click();
  await expect(accounts.nth(0)).toContainText('Glave');

  // Acme disconnected from its menu, Glave is DeepWiki's one account again, on one row.
  await accounts.nth(1).getByTestId('connector-account-menu-trigger').click();
  await byTestId(page, 'connector-account-menu-action').getByText('Disconnect').click();
  await byTestId(page, 'connectors-disconnect-ok').click();
  await expect(accounts).toHaveCount(0);
  await expect(connected.getByTestId('connector-detail')).toContainText(
    'Everywhere, including chats',
  );
  await connected.getByTestId('connector-action').click();
  await byTestId(page, 'connectors-disconnect-ok').click();
  await expect(deepwiki.getByTestId('connector-action')).toHaveText('Connect');
  expect(problems()).toEqual([]);
});
