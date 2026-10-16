import { readFileSync } from 'node:fs';
import { type IncomingMessage, type Server, type ServerResponse, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { join } from 'node:path';

import { expect, test } from '@playwright/test';

import {
  FIRST_RUN_HOME,
  FIRST_RUN_URL,
  byTestId,
  expectAccessible,
  watchForProblems,
} from './support';

/** The key the person pastes, and one the fake service refuses. */
const KEY = 'sk-e2e-first-run-0123456789wxyz';
const WRONG = 'sk-e2e-wrong-key-0123456789abcd';

/** Words a person on their own computer should never meet (AGENTS.md, "Keeping principle 4"). */
const ADMIN_WORDS = /policy|organization|member|group|admin|SSO|config\.toml|GLAVE_/u;

/** What the fake model service says to every chat. */
const REPLY = 'Hello from your own model.';

/** Answers as an OpenAI-compatible service would: its model list, and chats as streams. */
function serve(request: IncomingMessage, response: ServerResponse): void {
  const keyed = request.headers.authorization === `Bearer ${KEY}`;
  if (!keyed) {
    response.writeHead(401, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ error: { message: 'Incorrect API key provided' } }));
    return;
  }
  if (request.method === 'GET' && request.url === '/v1/models') {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ object: 'list', data: [{ id: 'fake-chat', created: 1 }] }));
    return;
  }
  request.resume();
  request.on('end', () => {
    const chunks = [
      { id: 'gen-1', model: 'fake-chat', choices: [{ index: 0, delta: { content: REPLY } }] },
      {
        choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
        usage: { prompt_tokens: 10, completion_tokens: 5 },
      },
    ];
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    response.end(
      `${chunks.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`).join('')}data: [DONE]\n\n`,
    );
  });
}

let service: Server;

test.beforeAll(async () => {
  service = createServer(serve);
  await new Promise<void>((resolve) => service.listen(0, '127.0.0.1', resolve));
});

test.afterAll(async () => {
  // glave keeps its connections to the service open for reuse, and closing would wait for them.
  const closed = new Promise((resolve) => service.close(resolve));
  service.closeAllConnections();
  await closed;
});

test('a first run adds a model in the browser, then chats with it', async ({ page }) => {
  const problems = watchForProblems(page);
  const home = process.env[FIRST_RUN_HOME] ?? '';
  const token = readFileSync(join(home, 'serve.token'), 'utf8').trim();
  const { port } = service.address() as AddressInfo;

  await page.goto(`${FIRST_RUN_URL}/#token=${token}`);
  await expect(page).toHaveURL(/\/connect$/u);
  await expect(page.getByRole('heading', { name: 'Add a model' })).toBeVisible();
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  await page.getByRole('radio', { name: 'Another server' }).check();
  await byTestId(page, 'connect-address').fill(`http://127.0.0.1:${String(port)}/v1`);
  await byTestId(page, 'connect-key').fill(WRONG);
  await byTestId(page, 'connect-submit').click();
  await expect(byTestId(page, 'connect-problem')).toContainText('did not accept that key');
  await expectAccessible(page);

  await byTestId(page, 'connect-key').fill(KEY);
  await byTestId(page, 'connect-submit').click();
  await expect(page).toHaveURL(`${FIRST_RUN_URL}/`);
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Hi');
  await prompt.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText(REPLY);

  await page.goto(`${FIRST_RUN_URL}/settings`);
  await expect(byTestId(page, 'settings-account')).toContainText(`127.0.0.1:${String(port)}`);
  await expect(byTestId(page, 'settings-account')).toContainText('Key ending wxyz');
  expect(await page.content()).not.toContain(KEY);
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);
  expect(problems()).toEqual([]);
});
