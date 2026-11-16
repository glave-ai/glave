import type { ScheduleInfo } from '@glave/client';

import { schedule } from '../testing/fixtures';
import { lastOf, nextOf, repeatLine, runAt } from './repeating';

const DUE = '2026-10-12T08:00:00.000Z';

function last(value: ScheduleInfo['last']): ScheduleInfo {
  return { ...schedule('s1', DUE), last: value };
}

describe('repeating', () => {
  it('writes a run’s time with its weekday, for the reader', () => {
    expect(runAt(DUE, 'en-US')).toMatch(/^Mon \d{1,2}:00\s?[AP]M$/);
  });

  it('says when a schedule runs next, that it is paused, or nothing once it runs no more', () => {
    expect(nextOf(schedule('s1', DUE), 'en-US')).toMatch(/^Next: Mon /);
    expect(nextOf({ ...schedule('s1', DUE), paused: true }, 'en-US')).toBe('Paused');
    expect(nextOf(schedule('s1', null), 'en-US')).toBeNull();
  });

  it('says how the last run went, by its status', () => {
    expect(lastOf(schedule('s1', DUE), 'en-US')).toBeNull();
    expect(lastOf(last({ status: 'running', due: DUE }), 'en-US')).toMatch(/, running$/);
    expect(lastOf(last({ status: 'skipped', due: DUE }), 'en-US')).toContain('skipped');
    const done = last({ status: 'ended', due: DUE, outcome: 'completed' });
    expect(lastOf(done, 'en-US')).toMatch(/, done$/);
    const failed = last({ status: 'ended', due: DUE, outcome: 'failed' });
    expect(lastOf(failed, 'en-US')).toMatch(/, didn't finish$/);
    expect(lastOf(last({ status: 'started', due: DUE }), 'en-US')).toMatch(/^Last: Mon /);
  });

  it('puts the server’s words first, then the next and last run', () => {
    const line = repeatLine(last({ status: 'running', due: DUE }), 'en-US');
    expect(line).toMatch(/^Every day at 2:00 am · Next: Mon .* · Last: Mon .*, running$/);
  });
});
