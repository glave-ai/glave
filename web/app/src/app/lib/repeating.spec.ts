import { schedule } from '../testing/fixtures';
import { type ScheduleReader, nextOf, repeatLine, runAt, runDate } from './repeating';

/** A reader in Lisbon, as the fixture's schedule is: its times name no zone. */
const LISBON: ScheduleReader = { locale: 'en-US', timeZone: 'Europe/Lisbon' };
/** A reader in New York, five hours behind Lisbon in November. */
const NEW_YORK: ScheduleReader = { locale: 'en-US', timeZone: 'America/New_York' };
/** Monday 16 November 2026, 9:00 in Lisbon. */
const NOW = Date.parse('2026-11-16T09:00:00.000Z');

describe('repeating', () => {
  it('says a run’s day as the reader would, near or far, and its time in the schedule’s zone', () => {
    expect(runAt('2026-11-17T08:00:00.000Z', 'Europe/Lisbon', LISBON, NOW)).toBe(
      'tomorrow, 8:00 AM',
    );
    expect(runAt('2026-11-16T08:00:00.000Z', 'Europe/Lisbon', LISBON, NOW)).toBe('today, 8:00 AM');
    expect(runAt('2026-11-14T19:00:00.000Z', 'Europe/Lisbon', LISBON, NOW)).toBe(
      'Saturday, 7:00 PM',
    );
    expect(runAt('2026-11-30T08:00:00.000Z', 'Europe/Lisbon', LISBON, NOW)).toBe('Nov 30, 8:00 AM');
    expect(runAt('2027-01-04T08:00:00.000Z', 'Europe/Lisbon', LISBON, NOW)).toBe(
      'Jan 4, 2027, 8:00 AM',
    );
    expect(runAt('soon', 'Europe/Lisbon', LISBON, NOW)).toBeNull();
  });

  it('names the schedule’s zone to a reader in another, and counts days on its clocks', () => {
    // 10 PM on Monday in New York is already Tuesday in Lisbon: the schedule's day is today.
    expect(runAt('2026-11-17T03:00:00.000Z', 'America/New_York', LISBON, NOW)).toBe(
      'today, 10:00 PM EST',
    );
    expect(runAt('2026-11-17T13:00:00.000Z', 'America/New_York', LISBON, NOW)).toBe(
      'tomorrow, 8:00 AM EST',
    );
    // A zone the platform does not know is read in the reader's own.
    expect(runAt('2026-11-17T13:00:00.000Z', 'Mars/Olympus', NEW_YORK, NOW)).toBe(
      'tomorrow, 8:00 AM',
    );
  });

  it('heads a run with its date and time in the schedule’s zone', () => {
    expect(runDate('2026-11-12T08:00:00.000Z', 'Europe/Lisbon', LISBON)).toBe(
      'Thursday, November 12 · 8:00 AM',
    );
    expect(runDate('2026-11-12T13:00:00.000Z', 'America/New_York', LISBON)).toBe(
      'Thursday, November 12 · 8:00 AM EST',
    );
    expect(runDate('soon', 'Europe/Lisbon', LISBON)).toBeNull();
  });

  it('says when a schedule runs next, that it is paused, or nothing once it runs no more', () => {
    const next = schedule('s1', '2026-11-17T02:00:00.000Z');
    expect(nextOf(next, LISBON, NOW)).toBe('Next: tomorrow, 2:00 AM');
    expect(nextOf({ ...next, paused: true }, LISBON, NOW)).toBe('Paused');
    expect(nextOf(schedule('s1', null), LISBON, NOW)).toBeNull();
  });

  it('puts the server’s words first in a chat’s line, then the next run and its note', () => {
    const next = schedule('s1', '2026-11-17T02:00:00.000Z');
    expect(repeatLine(next, LISBON, NOW)).toBe('Every day at 2:00 am · Next: tomorrow, 2:00 AM');
    const old = {
      ...schedule('s1', null),
      paused: true,
      note: 'It runs no more: set it up again.',
    };
    expect(repeatLine(old, LISBON, NOW)).toBe(
      'Every day at 2:00 am · Paused · It runs no more: set it up again.',
    );
  });
});
