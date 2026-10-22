import { type Page, type Route, expect, test } from '@playwright/test';

import { HELLO } from './fake-model';
import { LIVE, byTestId, served, signInCode } from './support';

/** Whether `route` is a `turn/start`. */
function turnStart(route: Route): boolean {
  return route.request().headers()['glave-method'] === 'turn/start';
}

/** The idempotency key of every `turn/start` `page` sends from now on. */
function keysSent(page: Page): readonly string[] {
  const keys: string[] = [];
  page.on('request', (request) => {
    if (request.headers()['glave-method'] === 'turn/start') {
      const { params } = request.postDataJSON() as {
        params: { _meta: { idempotency_key: string } };
      };
      keys.push(params._meta.idempotency_key);
    }
  });
  return keys;
}

/** Writes `prompt` in a new chat on the live glave, signed in, as this browser's own. */
async function write(page: Page, prompt: string, browserName: string): Promise<string> {
  const own = `${prompt} from ${browserName} at ${String(Date.now())}`;
  await page.goto(`${served(LIVE)}/#code=${await signInCode(LIVE)}`);
  await byTestId(page, 'home-composer-input').fill(own);
  return own;
}

/** Waits until the thread shows its turn answered, and says how often it shows `prompt`. */
async function asked(page: Page, prompt: string): Promise<number> {
  await expect(byTestId(page, 'thread-message').last()).toContainText(HELLO);
  return byTestId(page, 'thread-message').filter({ hasText: prompt }).count();
}

/**
 * A real glave, its model the fake of fake-model.ts: the new chat sends the first turn as the
 * person sends it and opens the thread at once; the thread's page only follows the server, so
 * coming back to it never sends again, and one click starts one turn.
 */
test('Back, forward and a reload after a first message start no second turn', async ({
  page,
  browserName,
}) => {
  const keys = keysSent(page);
  const prompt = await write(page, 'Back', browserName);
  await byTestId(page, 'home-composer-input').press('Enter');
  expect(await asked(page, prompt)).toBe(1);
  const thread = page.url();
  await page.goBack();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  await page.goForward();
  await expect(page).toHaveURL(thread);
  expect(await asked(page, prompt)).toBe(1);
  await page.reload();
  expect(await asked(page, prompt)).toBe(1);
  expect(keys).toHaveLength(1);
});

test('a double click on Send starts one turn', async ({ page, browserName }) => {
  const keys = keysSent(page);
  const prompt = await write(page, 'Twice', browserName);
  await byTestId(page, 'home-composer-send').dblclick();
  expect(await asked(page, prompt)).toBe(1);
  expect(keys).toHaveLength(1);
});

test('the thread opens with the message before the server has its turn', async ({
  page,
  browserName,
}) => {
  let letGo = (): void => undefined;
  const held = new Promise<void>((resolve) => {
    letGo = resolve;
  });
  await page.route(`${served(LIVE)}/`, async (route) => {
    if (turnStart(route)) {
      await held;
    }
    await route.fallback();
  });
  const prompt = await write(page, 'Early', browserName);
  await byTestId(page, 'home-composer-input').press('Enter');
  await expect(byTestId(page, 'thread-message').filter({ hasText: prompt })).toHaveCount(1);
  await expect(byTestId(page, 'thread-working')).toBeVisible();
  letGo();
  expect(await asked(page, prompt)).toBe(1);
});

test('a first message whose answer is lost is sent again with its key, as one turn', async ({
  page,
  browserName,
}) => {
  const keys = keysSent(page);
  let lose = true;
  await page.route(`${served(LIVE)}/`, async (route) => {
    if (lose && turnStart(route)) {
      lose = false;
      await route.abort('connectionreset');
      return;
    }
    await route.fallback();
  });
  const prompt = await write(page, 'Lost', browserName);
  await byTestId(page, 'home-composer-input').press('Enter');
  await expect(byTestId(page, 'thread-unsent')).toContainText('No answer came back.');
  await byTestId(page, 'thread-send-again').click();
  expect(await asked(page, prompt)).toBe(1);
  await expect(byTestId(page, 'thread-unsent')).toHaveCount(0);
  expect(keys).toHaveLength(2);
  expect(keys[1]).toBe(keys[0]);
});
