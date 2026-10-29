import { type IncomingMessage, type Server, type ServerResponse, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';

import {
  ADMIN_WORDS,
  ACCOUNTS,
  byTestId,
  expect,
  expectAccessible,
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
  await expect(page.getByRole('heading', { name: 'Connected accounts' })).toBeVisible();
  await expect(byTestId(page, 'accounts-none')).toBeVisible();
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }

  await byTestId(page, 'accounts-host-address').fill(address);
  await byTestId(page, 'accounts-host-token').fill(WRONG);
  await byTestId(page, 'accounts-host-submit').click();
  await expect(byTestId(page, 'accounts-host-problem')).toContainText('did not accept that token');
  await expectAccessible(page);

  await byTestId(page, 'accounts-host-token').fill(TOKEN);
  await byTestId(page, 'accounts-host-submit').click();
  const account = byTestId(page, 'accounts-account');
  await expect(account).toContainText(`127.0.0.1:${String(port)} (Gitea)`);
  await expect(account).toContainText('Signed in as ana, token ending wxyz');
  // Gitea does not say what a token may do, so nothing is claimed.
  await expect(byTestId(page, 'accounts-abilities-note')).toContainText(
    'depends on the access you gave its token',
  );
  await expect(byTestId(page, 'accounts-host-token')).toHaveValue('');

  await page.reload();
  await expect(byTestId(page, 'accounts-account')).toContainText('Signed in as ana');
  expect(await page.content()).not.toContain(TOKEN);
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);

  await byTestId(page, 'accounts-remove').click();
  await byTestId(page, 'accounts-remove-confirm-ok').click();
  await expect(byTestId(page, 'accounts-none')).toBeVisible();
  expect(problems()).toEqual([]);
});
