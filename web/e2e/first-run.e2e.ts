import { type IncomingMessage, type Server, type ServerResponse, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';

import {
  ADMIN_WORDS,
  FIRST_RUN,
  byTestId,
  expect,
  expectAccessible,
  served,
  signInCode,
  test,
  watchForProblems,
} from './support';

/** The key the person pastes, and one the fake service refuses. */
const KEY = 'fake-key-for-the-first-run-wxyz';
const WRONG = 'fake-key-the-service-refuses';

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
    const model = {
      id: 'fake-chat',
      created: 1,
      context_length: 32_768,
      pricing: { prompt: '0', completion: '0' },
    };
    response.end(JSON.stringify({ object: 'list', data: [model] }));
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
  const glave = served(FIRST_RUN);
  const code = await signInCode(FIRST_RUN);
  const { port } = service.address() as AddressInfo;

  await page.goto(`${glave}/#code=${code}`);
  await expect(page).toHaveURL(/\/connect$/u);
  await expect(page.getByRole('heading', { name: 'Add a model' })).toBeVisible();
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  // OpenRouter comes first; this run connects a server of its own.
  await expect(page.getByRole('radio', { name: 'OpenRouter' })).toBeChecked();
  await page.getByRole('radio', { name: 'Another server' }).check();
  await byTestId(page, 'connect-address').fill(`http://127.0.0.1:${String(port)}/v1`);
  await byTestId(page, 'connect-key').fill(WRONG);
  await byTestId(page, 'connect-submit').click();
  await expect(byTestId(page, 'connect-problem')).toContainText('did not accept that key');
  await expectAccessible(page);

  await byTestId(page, 'connect-key').fill(KEY);
  await byTestId(page, 'connect-submit').click();
  await expect(page).toHaveURL(`${glave}/`);
  const prompt = byTestId(page, 'home-composer-input');
  await prompt.fill('Hi');
  await prompt.press('Enter');
  await expect(byTestId(page, 'thread-message').last()).toContainText(REPLY);
  // The model as its service describes it.
  await byTestId(page, 'model-picker-trigger').click();
  await expect(byTestId(page, 'model-picker-option')).toContainText('33K context · Free');
  await page.keyboard.press('Escape');

  await page.goto(`${glave}/settings`);
  await expect(byTestId(page, 'settings-account')).toContainText(`127.0.0.1:${String(port)}`);
  await expect(byTestId(page, 'settings-account')).toContainText('1 model');
  await byTestId(page, 'settings-refresh-models').click();
  await expect(byTestId(page, 'settings-refreshed')).toContainText('up to date');
  await expectAccessible(page);
  expect(await page.content()).not.toContain(KEY);
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);
  expect(problems()).toEqual([]);
});
