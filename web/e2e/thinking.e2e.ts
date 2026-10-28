import { type Locator, type Page } from '@playwright/test';

import { THREAD, fakeApi } from './fake-api';
import { letGo } from './fake-gate';
import {
  ANSWER,
  HEADLINE,
  STARTED,
  STEP,
  THINKING,
  THINKING_PROMPT,
  thinkingTurn,
} from './fake-thinking';
import {
  SIGNED_IN,
  byTestId,
  expect,
  expectAccessible,
  expectEven,
  shownOpacity,
  test,
  watchForProblems,
} from './support';

/** A smithy's word, which the working line says while the model thinks. */
const SMITHY =
  /^(Forging|Honing|Tempering|Whetting|Hammering it out|Sharpening|Quenching|Polishing)…$/u;

/** The page's clock, held seven seconds after the turn started. */
const NOW = new Date(Date.parse(STARTED) + 7_000);

/** The model's reasoning, as written, without the blank lines at its ends. */
const REASONING = THINKING.join('').trim();

/** The motion tokens as the page reads them, in milliseconds: quick, standard and slow. */
async function durations(page: Page): Promise<readonly number[]> {
  return page.evaluate(() => {
    const root = getComputedStyle(document.documentElement);
    return ['quick', 'standard', 'slow'].map((name) => {
      // The build writes a time its shortest way: 120ms as .12s.
      const time = root.getPropertyValue(`--glz-duration-${name}`).trim();
      return Math.round(parseFloat(time) * (time.endsWith('ms') ? 1 : 1000));
    });
  });
}

/** How many animations run anywhere in the page. */
async function moving(page: Page): Promise<number> {
  return page.evaluate(() => document.getAnimations().length);
}

/** The animation an element runs, as its computed style names it. */
async function animationOf(locator: Locator): Promise<string> {
  return locator.evaluate((element) => getComputedStyle(element).animationName);
}

/** The animation the card of what changed runs as it appears, on the element that holds it. */
async function settling(page: Page): Promise<string> {
  return byTestId(page, 'turn-changes')
    .last()
    .evaluate((card) => getComputedStyle(card.parentElement ?? card).animationName);
}

/**
 * Shuts a run of calls by a click on its line, and says at that moment what slides in its fold
 * (each transition's property) and whether it is still open.
 */
async function shut(run: Locator): Promise<{ sliding: readonly string[]; open: boolean }> {
  return run.evaluate((host) => {
    const fold = host.querySelector('[data-testid="thread-run-fold"]');
    host.querySelector<HTMLElement>('[data-testid="thread-run-summary"]')?.click();
    return {
      sliding: (fold?.getAnimations() ?? []).map((each) =>
        each instanceof CSSTransition ? each.transitionProperty : each.id,
      ),
      open: host.querySelector('details')?.open ?? false,
    };
  });
}

/** Sends the thinking turn's prompt from the thread's composer. */
async function ask(page: Page): Promise<void> {
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  await expect(byTestId(page, 'thread-message').last()).toContainText('They pass now.');
  const composer = byTestId(page, 'thread-composer-input');
  await composer.fill(THINKING_PROMPT);
  await composer.press('Enter');
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`glave says what it does as it works, then the model's thinking, then the reply (${colorScheme})`, async ({
    page,
  }) => {
    const problems = watchForProblems(page);
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.clock.setFixedTime(NOW);
    await fakeApi(page);
    await thinkingTurn(page);
    await ask(page);
    expect(await durations(page)).toEqual([120, 200, 320]);

    // Before the server has recorded anything: the message, and a smithy's word under it, the
    // next every couple of seconds; no time yet to count from.
    const working = byTestId(page, 'thread-working');
    const label = byTestId(page, 'thread-working-label');
    const elapsed = byTestId(page, 'thread-working-elapsed');
    await expect(byTestId(page, 'thread-message').last()).toContainText(THINKING_PROMPT);
    await expect(label).toHaveText(SMITHY);
    await expect(elapsed).toHaveCount(0);
    const first = (await label.textContent()) ?? '';
    await expect(label).not.toHaveText(first);
    await expect(label).toHaveText(SMITHY);
    await expectAccessible(page);

    // Recorded: how long the turn has worked, from the server's start.
    await letGo(page, 'start');
    await expect(elapsed).toHaveText('7s');
    await expect(label).toHaveText(SMITHY);

    // A tool runs: the step in the server's own words.
    await letGo(page, 'tool');
    await expect(label).toHaveText(STEP);
    await expect(elapsed).toHaveText('7s');
    await expectAccessible(page);

    // The model's reasoning streams in as one line in the working line's place, rising into it:
    // "Thinking", shimmering, and its headline, the server's; what the tool changed settles in.
    await letGo(page, 'think');
    const live = byTestId(page, 'thread-reply');
    const thinking = live.getByTestId('thread-thought-line');
    await expect(thinking).toBeVisible();
    await expect(working).toHaveCount(0);
    expect(await animationOf(live)).toContain('glz-rise');
    // What changed comes from the server once the write has ended: the turn's own card.
    await expect(byTestId(page, 'turn-changes')).toHaveCount(2);
    expect(await settling(page)).toBe('glz-settle');
    await expect(live.getByTestId('thread-thought-label')).toHaveText('Thinking');
    expect(await animationOf(live.getByTestId('thread-thought-label'))).toContain('glz-shimmer');
    await expect(live.getByTestId('thread-thought-headline')).toHaveText(HEADLINE);
    await expect(live.locator('details')).toHaveCount(0);
    // Chosen, it shows the reasoning in full in the panel, and says so.
    const panel = byTestId(page, 'panel-thought-text');
    await expect(panel).toHaveCount(0);
    await thinking.click();
    await expect(panel).toHaveText(REASONING);
    await expect(thinking).toHaveAttribute('aria-pressed', 'true');
    await expectAccessible(page);

    // Once done it says how long it thought, its line kept and still chosen as its message is
    // recorded.
    await letGo(page, 'answer');
    const reply = byTestId(page, 'thread-message').last();
    await expect(reply).toContainText(ANSWER.join(''));
    await expect(byTestId(page, 'thread-composer-stop')).toHaveCount(0);
    const line = reply.getByTestId('thread-thought-line');
    await expect(reply.getByTestId('thread-thought-label')).toHaveText('Thought for 3s');
    await expect(reply.getByTestId('thread-thought-headline')).toHaveText(HEADLINE);
    await expect(line).toHaveAttribute('aria-pressed', 'true');
    await expect(panel).toHaveText(REASONING);
    await expect(byTestId(page, 'panel-thought-took')).toHaveText('3s');
    await expect(working).toHaveCount(0);
    // Chosen again, the panel shows it no longer. The click leaves focus on the line, but no
    // ring, so the reply's copy button goes with the pointer; a key pressed there (Enter, which
    // shows it again) gives focus its ring, and shows the button again.
    await line.click();
    await expect(panel).toHaveCount(0);
    await expect(line).toHaveAttribute('aria-pressed', 'false');
    const copy = reply.getByTestId('message-copy');
    await page.mouse.move(0, 0);
    await expect.poll(() => shownOpacity(copy)).toBe(0);
    await line.press('Enter');
    await expect(panel).toHaveText(REASONING);
    await expect.poll(() => shownOpacity(copy)).toBe(1);
    await expectAccessible(page);

    // The run of calls is the server's line, which slides open in its fold, and slides shut
    // before it closes.
    const run = byTestId(page, 'thread-run').last();
    const receipt = run.getByTestId('thread-receipt');
    await expect(run.getByTestId('thread-run-summary')).toHaveText('Ran 1 call');
    await run.getByTestId('thread-run-summary').click();
    await expect(receipt).toBeVisible();
    expect(await animationOf(run.getByTestId('thread-run-fold'))).toContain('glz-unfold');
    await expectAccessible(page);
    expect(await shut(run)).toEqual({ sliding: ['grid-template-rows'], open: true });
    await expect(receipt).toBeHidden();
    expect(problems()).toEqual([]);
  });
}

test('nothing moves when the person asks for less motion, though the words still change', async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await fakeApi(page);
  await thinkingTurn(page);
  await ask(page);
  const label = byTestId(page, 'thread-working-label');
  await expect(label).toHaveText(SMITHY);
  expect(await durations(page)).toEqual([0, 0, 0]);
  expect(await moving(page)).toBe(0);
  // The words are not motion: they change all the same.
  const first = (await label.textContent()) ?? '';
  await expect(label).not.toHaveText(first);
  await letGo(page, 'start');
  await letGo(page, 'tool');
  await expect(label).toHaveText(STEP);
  expect(await moving(page)).toBe(0);

  // The reply and what changed arrive in place, still.
  await letGo(page, 'think');
  const live = byTestId(page, 'thread-reply');
  await expect(live.getByTestId('thread-thought')).toBeVisible();
  expect(await animationOf(live)).toBe('none');
  // Its word holds still.
  expect(await animationOf(live.getByTestId('thread-thought-label'))).toBe('none');
  await expect(byTestId(page, 'turn-changes')).toHaveCount(2);
  expect(await settling(page)).toBe('none');
  expect(await moving(page)).toBe(0);
  await letGo(page, 'answer');
  await expect(byTestId(page, 'thread-message').last()).toContainText(ANSWER.join(''));

  // The run of calls opens and shuts at once.
  const run = byTestId(page, 'thread-run').last();
  const receipt = run.getByTestId('thread-receipt');
  await run.getByTestId('thread-run-summary').click();
  await expect(receipt).toBeVisible();
  expect(await animationOf(run.getByTestId('thread-run-fold'))).toBe('none');
  expect(await moving(page)).toBe(0);
  expect(await shut(run)).toEqual({ sliding: [], open: false });
  await expect(receipt).toBeHidden();
  expect(await moving(page)).toBe(0);
});

test('the working line moves again once the person no longer asks for less motion', async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await fakeApi(page);
  await thinkingTurn(page);
  await ask(page);
  const working = byTestId(page, 'thread-working');
  await expect(byTestId(page, 'thread-working-label')).toHaveText(SMITHY);
  const animations = (): Promise<number> =>
    working.evaluate((line) => line.getAnimations({ subtree: true }).length);
  expect(await animations()).toBe(0);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  // The mark breathes and the line shimmers.
  await expect.poll(animations).toBe(2);
});

test('the thinking in full, the steps and a step’s output sit evenly in their blocks', async ({
  page,
}) => {
  // Still, so each block is measured as it rests.
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await fakeApi(page);
  await thinkingTurn(page);
  await ask(page);
  await letGo(page, 'start');
  await letGo(page, 'tool');

  // The model's thinking ends in a blank line, which shows as no space under it in the panel,
  // as it streams and once recorded.
  await letGo(page, 'think');
  await byTestId(page, 'thread-reply').getByTestId('thread-thought-line').click();
  const thought = byTestId(page, 'panel-thought-text');
  await expect(thought).toHaveText(REASONING);
  await expectEven(thought);
  await letGo(page, 'answer');
  await expect(byTestId(page, 'thread-composer-stop')).toHaveCount(0);
  await expect(
    byTestId(page, 'thread-message').last().getByTestId('thread-thought-line'),
  ).toHaveAttribute('aria-pressed', 'true');
  await expectEven(thought);

  // A step's output ends in a blank line too, which its box leaves out; the run's rule runs
  // from the first step's line to the end of the last one's output.
  const run = byTestId(page, 'thread-run').last();
  await run.getByTestId('thread-run-summary').click();
  await run.getByTestId('thread-receipt-summary').click();
  const output = run.getByTestId('thread-output');
  await expect(output).toBeVisible();
  await expectEven(output);
  await expectEven(
    run.getByTestId('thread-run-steps'),
    run.getByTestId('thread-receipt-summary').or(output),
  );
});
