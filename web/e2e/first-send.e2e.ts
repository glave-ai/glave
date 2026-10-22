import { type Page, type Request, expect, test } from '@playwright/test';

import { HELLO } from './fake-model';
import { LIVE, byTestId, served, signInCode } from './support';

/** A `turn/start` the page sent: its thread and the idempotency key it carried. */
interface TurnStart {
  readonly thread: string;
  readonly key: string | undefined;
}

/** What `request` asked, when it is a `turn/start`. */
function turnStart(request: Request): TurnStart | null {
  if (request.headers()['glave-method'] !== 'turn/start') {
    return null;
  }
  const { params } = request.postDataJSON() as {
    params: { thread: string; _meta?: { idempotency_key?: string } };
  };
  return { thread: params.thread, key: params._meta?.idempotency_key };
}

/** Every `turn/start` `page` sends from now on, in order. */
function turnStarts(page: Page): readonly TurnStart[] {
  const sent: TurnStart[] = [];
  page.on('request', (request) => {
    const start = turnStart(request);
    if (start !== null) {
      sent.push(start);
    }
  });
  return sent;
}

/** Opens a new chat on the live glave, signed in, and writes `prompt` in its composer. */
async function write(page: Page, prompt: string): Promise<void> {
  await page.goto(`${served(LIVE)}/#code=${await signInCode(LIVE)}`);
  await byTestId(page, 'home-composer-input').fill(prompt);
}

/** Waits until the thread page shows `prompt`'s turn, answered, and says how often it shows it. */
async function asked(page: Page, prompt: string): Promise<number> {
  await expect(page).toHaveURL(/\/t\/[0-9a-f-]+$/u);
  await expect(byTestId(page, 'thread-message').last()).toContainText(HELLO);
  return byTestId(page, 'thread-message').filter({ hasText: prompt }).count();
}

/** A prompt of this test's own: the browsers run side by side on the one live glave. */
function promptFor(test: string, browserName: string): string {
  return `${test} from ${browserName} at ${String(Date.now())}`;
}

/**
 * A real glave, its model the fake service of fake-model.ts: the first message is sent as the
 * person sends it, from the new chat, which opens the thread once the server has the message;
 * the thread's page only shows and follows what the server has, so coming back to it never
 * sends again.
 */
test('Back, forward and a reload after a first message start no second turn', async ({
  page,
  browserName,
}) => {
  const sent = turnStarts(page);
  const prompt = promptFor('Back', browserName);
  await write(page, prompt);
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
  expect(sent).toHaveLength(1);
});

test('a double click on Send starts one turn', async ({ page, browserName }) => {
  const sent = turnStarts(page);
  const prompt = promptFor('Twice', browserName);
  await write(page, prompt);
  await byTestId(page, 'home-composer-send').dblclick();
  expect(await asked(page, prompt)).toBe(1);
  expect(new Set(sent.map(({ key }) => key)).size).toBe(1);
});

test('a first message whose answer is lost, sent again, starts one turn', async ({
  page,
  browserName,
}) => {
  const sent = turnStarts(page);
  // The first `turn/start` reaches glave, which starts the turn, but its answer never arrives.
  let lose = true;
  await page.route(`${served(LIVE)}/`, async (route) => {
    if (lose && turnStart(route.request()) !== null) {
      lose = false;
      await route.fetch();
      await route.abort('connectionreset');
      return;
    }
    await route.fallback();
  });
  const prompt = promptFor('Lost', browserName);
  await write(page, prompt);
  const composer = byTestId(page, 'home-composer-input');
  await composer.press('Enter');
  await expect(byTestId(page, 'home-error')).toBeVisible();
  // Back in the box, sent again as it was: the same send.
  await expect(composer).toHaveValue(prompt);
  await composer.press('Enter');
  expect(await asked(page, prompt)).toBe(1);
  expect(sent).toHaveLength(2);
  expect(sent[1]).toEqual(sent[0]);
  expect(sent[0]?.key).toMatch(/^[0-9a-f]{32}$/u);
});
