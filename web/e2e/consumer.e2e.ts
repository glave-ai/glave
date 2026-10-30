import { type IncomingMessage, type Server, type ServerResponse, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';

import type { Page } from '@playwright/test';

import { COMMAND, RAN } from './fake-model';
import {
  ADMIN_WORDS,
  CONSUMER,
  LIVE_MODEL_URL,
  byTestId,
  expect,
  expectAccessible,
  served,
  signInCode,
  test,
  watchForProblems,
} from './support';

/** The token the person pastes for GitHub. */
const TOKEN = 'e2e0consumer0journey0wxyz';

/**
 * Answers as GitHub Enterprise Server's API does: whom a token is for at `/api/v3/user`, with
 * the scopes it was given, and nothing at GitLab's or Gitea's paths.
 */
function serve(request: IncomingMessage, response: ServerResponse): void {
  if (request.url !== '/api/v3/user' || request.headers.authorization !== `Bearer ${TOKEN}`) {
    response.writeHead(404, { 'content-type': 'application/json' });
    response.end('{}');
    return;
  }
  response.writeHead(200, { 'content-type': 'application/json', 'x-oauth-scopes': 'repo' });
  response.end(JSON.stringify({ login: 'ana' }));
}

let github: Server;

test.beforeAll(async () => {
  github = createServer(serve);
  await new Promise<void>((resolve) => github.listen(0, '127.0.0.1', resolve));
});

test.afterAll(async () => {
  const closed = new Promise((resolve) => github.close(resolve));
  github.closeAllConnections();
  await closed;
});

/** Fails if `page` shows an admin word, or fails axe, in light or in dark. */
async function expectPlain(page: Page): Promise<void> {
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
}

/**
 * A person's whole journey on their own computer, on a real glave with a fresh home: they add a
 * model, chat with it, approve a command it asks to run, and connect GitHub, and nowhere on the
 * way meet a word of an organization's (AGENTS.md, "Keeping principle 4").
 */
test('a person adds a model, chats, approves a command and connects GitHub, in plain words', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  const glave = served(CONSUMER);

  // Add a model.
  await page.goto(`${glave}/#code=${await signInCode(CONSUMER)}`);
  await expect(page).toHaveURL(/\/connect$/u);
  await expectPlain(page);
  await page.getByRole('radio', { name: 'Another server' }).check();
  await byTestId(page, 'connect-address').fill(process.env[LIVE_MODEL_URL] ?? '');
  await byTestId(page, 'connect-key').fill('fake-key-for-the-journey');
  await byTestId(page, 'connect-submit').click();
  await expect(page).toHaveURL(`${glave}/`);
  await expectPlain(page);

  // Chat, asking first before every change, and approve the command the model asks to run.
  await byTestId(page, 'permissions-trigger').click();
  await expectPlain(page);
  await byTestId(page, 'permissions-option').filter({ hasText: 'Ask first' }).click();
  await expect(byTestId(page, 'permissions-trigger')).toContainText('Ask first');
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Please run a command');
  await prompt.press('Enter');
  const card = byTestId(page, 'thread-approval');
  await expect(card).toContainText(COMMAND);
  await expectPlain(page);
  await byTestId(page, 'thread-approval-allow-once').click();
  await expect(byTestId(page, 'thread-message').last()).toContainText(RAN);
  await byTestId(page, 'model-picker-trigger').click();
  await expect(byTestId(page, 'model-picker-option')).toContainText('Fake Live');
  await expectPlain(page);
  await page.keyboard.press('Escape');

  // Connect GitHub, at an address of its own as GitHub Enterprise has: another service.
  await page.goto(`${glave}/settings/connections`);
  await expectPlain(page);
  const { port } = github.address() as AddressInfo;
  await byTestId(page, 'another-address').fill(`http://127.0.0.1:${String(port)}`);
  await byTestId(page, 'another-add').click();
  await byTestId(page, 'another-token').fill(TOKEN);
  await byTestId(page, 'another-connect').click();
  const account = byTestId(page, 'git-host');
  await expect(account).toContainText(`127.0.0.1:${String(port)} (GitHub)`);
  await expect(account).toContainText('Signed in as ana');
  await expect(account).not.toContainText('token ending');
  await expectPlain(page);

  // The rest of what a person reaches from the side nav and settings.
  for (const path of ['/tasks', '/memory', '/settings/connections']) {
    await page.goto(`${glave}${path}`);
    await expect(page.locator('h1')).toBeVisible();
    await expectPlain(page);
  }
  expect(await page.content()).not.toContain(TOKEN);
  expect(problems()).toEqual([]);
});
