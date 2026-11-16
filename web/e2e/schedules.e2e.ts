import { fakeApi } from './fake-api';
import { ASKING, MORNING, ZONE, schedules } from './fake-schedules';
import { SIGNED_IN, byTestId, expect, expectAccessible, test, watchForProblems } from './support';

test.use({ timezoneId: ZONE });

test('a repeat confirmed in a chat folds to a line to its own chat, and pauses from Tasks', async ({
  page,
}) => {
  const problems = watchForProblems(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await fakeApi(page);
  const server = await schedules(page);
  await page.goto(`/t/${ASKING}${SIGNED_IN}`);

  // The model asks in the server's words; the card offers Repeat it and Not now.
  const card = byTestId(page, 'thread-repeat');
  await expect(card).toContainText('Repeat this every weekday at 8:00 am?');
  await expect(byTestId(page, 'thread-repeat-lines').locator('li')).toHaveText([
    'First run: Tuesday 17 November at 8:00 am, your time.',
    'Each run is added to its own chat, “Morning email summary”.',
  ]);
  await expect(byTestId(page, 'thread-repeat-caution')).toHaveText(
    'It runs only while glave runs on this computer.',
  );
  await expect(card.getByRole('button')).toHaveText(['Repeat it', 'Not now']);
  await expectAccessible(page);
  await byTestId(page, 'thread-repeat-allow-once').click();

  // It folds to one line, which opens the schedule's own chat.
  const made = byTestId(page, 'thread-repeat-chat');
  await expect(made).toHaveText('Repeats every weekday at 8:00 am');
  await expect(card).toHaveCount(0);
  await made.click();
  await expect(page).toHaveURL(new RegExp(`/t/${MORNING}$`, 'u'));
  await expect(byTestId(page, 'thread-repeats')).toContainText('Every weekday at 8:00 am · Next:');
  await expect(byTestId(page, 'turn-ran')).toHaveText('Monday, November 16 · 8:00 AM');
  await expectAccessible(page);

  // Tasks lists it under Repeating, above what runs now, and pauses it.
  await byTestId(page, 'nav-tasks').click();
  const row = byTestId(page, 'repeating-schedule');
  await expect(row.getByTestId('repeating-name')).toHaveText('Morning email summary');
  await expect(row.getByTestId('repeating-line')).toContainText('Every weekday at 8:00 am · Next:');
  await expect(page.locator('main section > h2')).toHaveText(['Repeating', 'Running now']);
  await expectAccessible(page);
  await row.getByTestId('repeating-pause').click();
  await expect(row.getByTestId('repeating-line')).toContainText(
    'Every weekday at 8:00 am · Paused',
  );
  await expect(row.getByTestId('repeating-pause')).toHaveText('Resume');
  // The browser gave the server its time zone, once.
  expect(server.zones).toEqual([ZONE]);
  expect(problems()).toEqual([]);
});
