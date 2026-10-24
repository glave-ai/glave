import { type Page, type Request, expect, test } from '@playwright/test';

import { HELLO, LEFT_ALONE, TOUCH, holding, letGo } from './fake-model';
import {
  LIVE,
  LIVE_MODEL_URL,
  byTestId,
  expectAccessible,
  served,
  signInCode,
  watchForProblems,
} from './support';

/** The session API's methods `page` calls from now on, in order, by the header that names each. */
function callsOf(page: Page): readonly string[] {
  const called: string[] = [];
  page.on('request', (request) => {
    const method = request.headers()['glave-method'];
    if (method !== undefined) {
      called.push(method);
    }
  });
  return called;
}

/** Diagnostic for #695: every request of `page`, as it starts, answers and ends. */
function track(page: Page, name: string, log: string[], t0: number): Set<Request> {
  const open = new Set<Request>();
  const label = (request: Request): string =>
    request.headers()['glave-method'] ?? new URL(request.url()).pathname;
  const at = (): string => String(Date.now() - t0).padStart(6);
  page.on('request', (request) => {
    open.add(request);
    log.push(`${at()} ${name} > ${request.method()} ${label(request)}`);
  });
  page.on('response', (response) => {
    log.push(`${at()} ${name} < ${String(response.status())} ${label(response.request())}`);
  });
  page.on('requestfinished', (request) => {
    open.delete(request);
    log.push(`${at()} ${name} . done ${label(request)}`);
  });
  page.on('requestfailed', (request) => {
    open.delete(request);
    log.push(`${at()} ${name} x fail ${label(request)} ${request.failure()?.errorText ?? ''}`);
  });
  page.on('console', (message) => {
    log.push(`${at()} ${name} console ${message.type()} ${message.text()}`);
  });
  page.on('load', () => log.push(`${at()} ${name} LOAD`));
  page.on('domcontentloaded', () => log.push(`${at()} ${name} DOMCONTENTLOADED`));
  return open;
}

/** Diagnostic for #695: what each document sees of its visibility. */
async function seeVisibility(page: Page): Promise<void> {
  await page.addInitScript(() => {
    console.log(`diag visibility at start ${document.visibilityState}`);
    document.addEventListener('visibilitychange', () => {
      console.log(`diag visibility now ${document.visibilityState}`);
    });
    addEventListener('pagehide', () => console.log('diag pagehide'));
  });
}

/**
 * A real glave, its model the fake service of fake-model.ts, asking before commands run: what
 * one tab does in a thread, another tab open on it sees as it happens, the approval a turn
 * waits for among it, and either tab may answer.
 */
test('a second tab sees a turn and its approval as they happen, and answers it', async ({
  browser,
}) => {
  const context = await browser.newContext();
  const [first, second] = [await context.newPage(), await context.newPage()];
  const problems = [watchForProblems(first), watchForProblems(second)];
  const live = served(LIVE);

  await first.goto(`${live}/#code=${await signInCode(LIVE)}`);
  const prompt = byTestId(first, 'home-composer-input');
  await prompt.fill('Hi');
  await prompt.press('Enter');
  await expect(first).toHaveURL(/\/t\/[0-9a-f-]+$/u);
  await expect(byTestId(first, 'thread-message').last()).toContainText(HELLO);

  // The second tab opens the thread the first started, and follows it from then on.
  const thread = new URL(first.url()).pathname;
  await second.goto(`${live}${thread}#code=${await signInCode(LIVE)}`);
  await expect(byTestId(second, 'thread-message').last()).toContainText(HELLO);

  const reply = byTestId(first, 'thread-composer-input');
  await reply.fill('Please touch a file');
  await reply.press('Enter');
  const asked = byTestId(second, 'thread-message').filter({ hasText: 'Please touch a file' });
  await expect(asked).toBeVisible();
  const card = byTestId(second, 'thread-approval');
  await expect(card).toContainText('Needs you');
  await expect(card.locator('code')).toHaveText(TOUCH);
  for (const colorScheme of ['light', 'dark'] as const) {
    await second.emulateMedia({ colorScheme });
    await expectAccessible(second);
  }

  // Answered in the second tab, the turn goes on, and both tabs see how.
  await byTestId(second, 'thread-approval-deny').click();
  for (const page of [first, second]) {
    await expect(byTestId(page, 'thread-message').last()).toContainText(LEFT_ALONE);
  }
  // The first tab saw the decision too, in the turn's steps once it ended.
  await expect(byTestId(first, 'thread-approval-decided')).toHaveText('Denied');
  for (const seen of problems) {
    expect(seen()).toEqual([]);
  }
  await context.close();
});

/**
 * The side nav follows the lists live: a turn another tab starts shows running the moment it
 * runs (its model held until the test lets it go), then needing the person the moment it asks,
 * then done once answered, from the lists' watch alone, with no list read again on a timer.
 */
test('the side nav shows a turn another tab starts running, then needing the person, as it happens', async ({
  browser,
  browserName,
}) => {
  const context = await browser.newContext();
  const [watching, working] = [await context.newPage(), await context.newPage()];
  const t0 = Date.now();
  const log: string[] = [];
  const openA = track(watching, 'A', log, t0);
  const openB = track(working, 'B', log, t0);
  await seeVisibility(watching);
  await seeVisibility(working);
  const names = (open: Set<Request>): string =>
    [...open].map((r) => r.headers()['glave-method'] ?? new URL(r.url()).pathname).join(',');
  const problems = [watchForProblems(watching), watchForProblems(working)];
  const called = callsOf(watching);
  const live = served(LIVE);
  // A prompt of its own in each browser, which run side by side on the one glave.
  const held = `${browserName}-${String(Date.now())}-${String(test.info().repeatEachIndex)}`;
  const prompt = `${holding(held)}, then touch a file`;

  await watching.goto(`${live}/#code=${await signInCode(LIVE)}`);
  await expect.poll(() => called.includes('thread/list/watch')).toBe(true);
  const read = called.length;
  log.push(`${String(Date.now() - t0).padStart(6)} -- B goto; A holds [${names(openA)}]`);
  try {
    await working.goto(`${live}/#code=${await signInCode(LIVE)}`, { timeout: 30_000 });
  } catch (error) {
    const state = await working
      .evaluate(() => ({
        ready: document.readyState,
        visible: document.visibilityState,
        resources: performance
          .getEntriesByType('resource')
          .map((entry) => `${entry.name} ${String(Math.round(entry.duration))}`),
      }))
      .catch((failed: unknown) => String(failed));
    const stateA = await watching
      .evaluate(() => ({ ready: document.readyState, visible: document.visibilityState }))
      .catch((failed: unknown) => String(failed));
    console.log(`DIAG ${test.info().title} repeat ${String(test.info().repeatEachIndex)}`);
    console.log(log.join('\n'));
    console.log(`DIAG OPEN A: [${names(openA)}]`);
    console.log(`DIAG OPEN B: [${names(openB)}]`);
    console.log(`DIAG B state: ${JSON.stringify(state, null, 1)}`);
    console.log(`DIAG A state: ${JSON.stringify(stateA)}`);
    throw error;
  }
  log.push(
    `${String(Date.now() - t0).padStart(6)} -- B loaded; A [${names(openA)}] B [${names(openB)}]`,
  );
  console.log(`DIAG pass: ${log.filter((line) => line.includes(' -- ')).join(' | ')}`);
  const composer = byTestId(working, 'home-composer-input');
  await composer.fill(prompt);
  await composer.press('Enter');

  const row = byTestId(watching, 'nav-thread').filter({ hasText: prompt });
  await expect(row.getByTestId('nav-thread-running')).toBeVisible();
  await letGo(process.env[LIVE_MODEL_URL] ?? '', held);
  const waits = row.getByTestId('nav-thread-line');
  await expect(waits).toHaveText('Waiting on you');
  await expect(byTestId(working, 'thread-approval').locator('code')).toHaveText(TOUCH);
  for (const colorScheme of ['light', 'dark'] as const) {
    await watching.emulateMedia({ colorScheme });
    await expectAccessible(watching);
  }

  await byTestId(working, 'thread-approval-deny').click();
  await expect(row.getByTestId('nav-thread-since')).toBeVisible();
  await expect(waits).toHaveCount(0);
  // Heard from the lists' watch alone: nothing read the lists again.
  const reads = called.slice(read).filter((method) => /^(project|thread)\/list$/u.test(method));
  expect(reads).toEqual([]);
  for (const seen of problems) {
    expect(seen()).toEqual([]);
  }
  await context.close();
});
