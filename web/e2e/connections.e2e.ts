import { type ChildProcessByStdio, spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import type { Readable, Writable } from 'node:stream';

import { expect, test } from '@playwright/test';

import {
  CONNECTIONS,
  CONNECTIONS_WEBKIT,
  TEST_SERVER,
  byTestId,
  expectAccessible,
  homeOf,
  served,
  signInCode,
  watchForProblems,
} from './support';

/** The token the service's sign-in grants, which it lets in. */
const TOKEN = 'e2e-connections';

/**
 * Words a person on their own computer should never meet (AGENTS.md, "Keeping principle 4"),
 * nor the name of the protocol or the file their services are set up in.
 */
const ADMIN_WORDS = /policy|organization|member|group|admin|SSO|config\.toml|GLAVE_|MCP|mcp\.json/u;

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
  let letGo: () => void = () => undefined;
  const held = new Promise<void>((resolve) => {
    letGo = resolve;
  });
  await context.route(
    (url) => url.pathname === '/authorize',
    async (route) => {
      await held;
      await route.continue();
    },
  );
  const opened = page.waitForEvent('popup');
  await byTestId(page, 'connection-action').click();
  const tab = await opened;
  const waiting = byTestId(page, 'connection-waiting');
  await expect(waiting).toContainText('Finish signing in to Tracker in the new tab.');
  await expect(byTestId(page, 'connection-open')).toHaveAttribute('href', /\/authorize\?/u);
  await expect(byTestId(page, 'connection-action')).toHaveText('Cancel');
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  letGo();
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
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);

  await byTestId(page, 'connection-action').click();
  await byTestId(page, 'connections-disconnect-ok').click();
  await expect(available.getByTestId('connection')).toContainText('Connect');
  await expect(ready).toHaveCount(0);
  expect(problems()).toEqual([]);
});
