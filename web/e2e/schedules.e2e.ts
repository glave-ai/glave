import { fakeApi } from './fake-api';
import { ASKING, MORNING, REPEATS, ZONE, schedules } from './fake-schedules';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

/** Each width the card is tried at: beside a wide page, and on a phone. */
const VIEWS = [
  { name: 'wide', viewport: { width: 1440, height: 900 } },
  { name: 'phone', viewport: { width: 390, height: 844 } },
] as const;

for (const view of VIEWS) {
  test.describe(`on a ${view.name} screen`, () => {
    test.use({ timezoneId: ZONE, viewport: view.viewport });

    test('the server’s card to repeat work folds, once made, to its line, which opens the schedule’s chat', async ({
      page,
    }) => {
      const problems = watchForProblems(page);
      await fakeApi(page);
      const server = await schedules(page);
      await page.goto(`/t/${ASKING}${SIGNED_IN}`);
      const card = byTestId(page, 'thread-repeat');
      await expect(card).toContainText('Repeat this every weekday at 8:00 am?');
      const lines = byTestId(page, 'thread-repeat-lines');
      await expect(lines).toContainText(
        'Monday to Friday, 8:00 am your time. First run: Tuesday 17 November.',
      );
      await expect(lines).toContainText(
        'Each run is added to its own chat, “Morning email summary”.',
      );
      await expect(lines).toContainText('Each run starts with: “Summarize my email”');
      await expect(byTestId(page, 'thread-repeat-caution')).toHaveText(
        'It runs only while glave runs on this computer.',
      );
      const allow = byTestId(page, 'thread-repeat-allow-once');
      const deny = byTestId(page, 'thread-repeat-deny');
      await expect(allow).toHaveText('Repeat it');
      await expect(deny).toHaveText('Not now');
      // Side by side at every width, the primary first.
      const [yes, no] = [await allow.boundingBox(), await deny.boundingBox()];
      expect(Math.round(yes?.y ?? -1)).toBe(Math.round(no?.y ?? -2));
      expect((yes?.x ?? Infinity) < (no?.x ?? 0)).toBe(true);
      await expectAccessible(page);
      await allow.click();
      // The line is the server's words for the chat it made, when its schedule repeats.
      const line = byTestId(page, 'thread-repeat-chat');
      await expect(line).toHaveText(REPEATS);
      await expect(card).toHaveCount(0);
      await line.click();
      await expect(page).toHaveURL(new RegExp(`/t/${MORNING}$`, 'u'));
      // The browser gave the server its time zone, once.
      expect(server.zones).toEqual([ZONE]);
      expect(problems()).toEqual([]);
    });

    test('a card for a shape with no plain words is the same card, and folds the same', async ({
      page,
    }) => {
      await fakeApi(page);
      await schedules(page, { card: 'unplain' });
      await page.goto(`/t/${ASKING}${SIGNED_IN}`);
      const card = byTestId(page, 'thread-repeat');
      await expect(card).toContainText('Repeat this, next on Monday 30 November');
      await expect(byTestId(page, 'thread-repeat-lines')).toContainText(
        'Times are in your time zone.',
      );
      await expect(byTestId(page, 'thread-repeat-allow-thread')).toHaveCount(0);
      await expect(byTestId(page, 'thread-approval')).toHaveCount(0);
      await expectAccessible(page);
      await byTestId(page, 'thread-repeat-allow-once').click();
      await expect(byTestId(page, 'thread-repeat-chat')).toHaveText(REPEATS);
    });

    test('Not now folds the card away, and makes nothing', async ({ page }) => {
      await fakeApi(page);
      const server = await schedules(page);
      await page.goto(`/t/${ASKING}${SIGNED_IN}`);
      await byTestId(page, 'thread-repeat-deny').click();
      await expect(byTestId(page, 'thread-repeat')).toHaveCount(0);
      await expect(byTestId(page, 'thread-message').last()).toContainText('All right.');
      await expect(byTestId(page, 'thread-repeat-made')).toHaveCount(0);
      expect(server.decision).toBe('deny');
    });

    test('a schedule’s chat says when it repeats under its title, heads each run with its date, and pauses and goes from its bar', async ({
      page,
    }) => {
      const phone = view.name === 'phone';
      const problems = watchForProblems(page);
      await fakeApi(page);
      const server = await schedules(page, { existing: true });
      await page.goto(`/t/${MORNING}${SIGNED_IN}`);
      const line = byTestId(page, 'thread-repeats');
      // On a phone, when it repeats alone, its actions and the panel's under a ⋯.
      await expect(line).toContainText(phone ? REPEATS : `${REPEATS} · Next:`);
      await expect(byTestId(page, 'thread-repeats-menu-trigger')).toHaveCount(phone ? 1 : 0);
      await expect(byTestId(page, 'turn-ran')).toHaveText([
        'Friday, November 13 · 8:00 AM',
        'Monday, November 16 · 8:00 AM',
      ]);
      // Each run's prompt folds under its heading, as the server shows it, never the person's.
      const asked = byTestId(page, 'turn-asked').first();
      await expect(byTestId(page, 'turn-asked-summary').first()).toContainText('What it was asked');
      await byTestId(page, 'turn-asked-summary').first().click();
      await expect(asked).toContainText('Summarize my email');
      await expect(byTestId(page, 'thread-message').filter({ hasText: 'Summarize' })).toHaveCount(
        0,
      );
      if (!phone) {
        await expect(byTestId(page, 'nav-thread-repeats')).toHaveCount(1);
      }
      await expectAccessible(page);
      const act = async (action: 'pause' | 'remove'): Promise<void> => {
        if (phone) {
          await byTestId(page, 'thread-repeats-menu-trigger').click();
          await byTestId(page, 'thread-repeats-menu-action')
            .nth(action === 'pause' ? 0 : 1)
            .click();
        } else {
          await byTestId(page, `thread-repeats-${action}`).click();
        }
      };
      await act('pause');
      await expect.poll(() => server.paused).toBe(true);
      if (phone) {
        await byTestId(page, 'thread-repeats-menu-trigger').click();
        await expect(byTestId(page, 'thread-repeats-menu-action').first()).toHaveText('Resume');
        await page.keyboard.press('Escape');
      } else {
        await expect(line).toContainText(`${REPEATS} · Paused`);
        await expect(byTestId(page, 'thread-repeats-pause')).toHaveText('Resume');
      }
      await act('remove');
      // Removing asks in the server's words, naming it.
      await expect(byTestId(page, 'thread-repeats-confirm')).toContainText(
        'Stop repeating Morning email summary?',
      );
      await byTestId(page, 'thread-repeats-confirm-ok').click();
      await expect(line).toHaveCount(0);
      expect(server.removed).toBe(true);
      expect(problems()).toEqual([]);
    });

    test('Tasks lists what repeats above what runs now, and pauses and removes it', async ({
      page,
    }) => {
      const phone = view.name === 'phone';
      const problems = watchForProblems(page);
      await fakeApi(page);
      const server = await schedules(page, { existing: true });
      await page.goto(`/tasks${SIGNED_IN}`);
      await expect(byTestId(page, 'repeating-name')).toHaveText('Morning email summary');
      const parts = byTestId(page, 'repeating-part');
      // The last run's words are the server's, a part of their own.
      await expect(parts).toHaveText([REPEATS, /^Next: /u, /^Last: /u, 'Done']);
      await expect(byTestId(page, 'tasks-running')).toHaveText('Running now');
      // On a phone, a line for each part; beside a wide page, one line.
      const [first, second] = [await parts.nth(0).boundingBox(), await parts.nth(1).boundingBox()];
      expect((first?.y ?? 0) < (second?.y ?? 0)).toBe(phone);
      await expectAccessible(page);
      await byTestId(page, 'repeating-pause').click();
      await expect(parts.nth(1)).toHaveText('Paused');
      await expect(byTestId(page, 'repeating-pause')).toHaveText('Resume');
      expect(server.paused).toBe(true);
      await byTestId(page, 'repeating-remove').click();
      await expect(byTestId(page, 'repeating-confirm')).toContainText(
        'Stop repeating Morning email summary?',
      );
      await byTestId(page, 'repeating-confirm-ok').click();
      await expect(byTestId(page, 'repeating-none')).toHaveText('Nothing repeats now.');
      await expect(byTestId(page, 'repeating-heading')).toBeFocused();
      expect(server.removed).toBe(true);
      expect(problems()).toEqual([]);
    });

    test('a repeat the server refuses after the yes folds away, its call saying how it went', async ({
      page,
    }) => {
      await fakeApi(page);
      await schedules(page, { outcome: 'refused' });
      await page.goto(`/t/${ASKING}${SIGNED_IN}`);
      await byTestId(page, 'thread-repeat-allow-once').click();
      await expect(byTestId(page, 'thread-repeat')).toHaveCount(0);
      await expect(byTestId(page, 'thread-message').last()).toContainText('All right.');
      await expect(byTestId(page, 'thread-repeat-made')).toHaveCount(0);
      await expect(byTestId(page, 'thread-run-summary')).toContainText('Set up a schedule');
      await expectAccessible(page);
    });
  });
}
