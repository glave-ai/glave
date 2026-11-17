import { fakeApi } from './fake-api';
import { ASKING, MORNING, REFUSED, REPEATS, ZONE, schedules } from './fake-schedules';
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
      await expect(byTestId(page, 'thread-repeat-lines')).toContainText(
        'Monday to Friday, 8:00 am your time. First run: Tuesday 17 November.',
      );
      await expect(byTestId(page, 'thread-repeat-lines')).toContainText(
        'Each run is added to its own chat, “Morning email summary”.',
      );
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
      const line = byTestId(page, 'thread-repeat-chat');
      await expect(line).toHaveText(REPEATS);
      await expect(card).toHaveCount(0);
      await line.click();
      await expect(page).toHaveURL(new RegExp(`/t/${MORNING}$`, 'u'));
      // The browser gave the server its time zone, once.
      expect(server.zones).toEqual([ZONE]);
      expect(problems()).toEqual([]);
    });

    test('Not now folds the card away, and makes nothing', async ({ page }) => {
      await fakeApi(page);
      const server = await schedules(page);
      await page.goto(`/t/${ASKING}${SIGNED_IN}`);
      await byTestId(page, 'thread-repeat-deny').click();
      await expect(byTestId(page, 'thread-repeat')).toHaveCount(0);
      await expect(byTestId(page, 'thread-message').last()).toContainText('All right.');
      await expect(byTestId(page, 'thread-repeat-made')).toHaveCount(0);
      await expect(byTestId(page, 'thread-repeat-refused')).toHaveCount(0);
      expect(server.decision).toBe('deny');
    });

    test('a repeat the server refuses after the yes says why, in its words', async ({ page }) => {
      await fakeApi(page);
      await schedules(page, 'refused');
      await page.goto(`/t/${ASKING}${SIGNED_IN}`);
      await byTestId(page, 'thread-repeat-allow-once').click();
      await expect(byTestId(page, 'thread-repeat-refused')).toHaveText(REFUSED);
      await expect(byTestId(page, 'thread-repeat-chat')).toHaveCount(0);
      await expectAccessible(page);
    });
  });
}
