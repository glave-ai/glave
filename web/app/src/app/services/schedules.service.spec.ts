import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { schedule } from '../testing/fixtures';
import { SchedulesService } from './schedules.service';
import { TASKS_REFRESH_MS } from './tasks.service';

describe('SchedulesService', () => {
  it('pauses, resumes and removes a schedule, answering as the server has it', async () => {
    const fake = new FakeSession().answer('schedule/pause', (params) => {
      const { paused } = params as { readonly paused: boolean };
      return { schedule: { ...schedule('s1', null), paused } };
    });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const schedules = TestBed.inject(SchedulesService);
    expect((await schedules.pause('s1', true)).paused).toBe(true);
    expect((await schedules.pause('s1', false)).paused).toBe(false);
    await schedules.remove('s1');
    expect(fake.paramsOf('schedule/pause')).toEqual([
      { schedule: 's1', paused: true },
      { schedule: 's1', paused: false },
    ]);
    expect(fake.paramsOf('schedule/delete')).toEqual([{ schedule: 's1' }]);
  });

  it('throws what the server refused a change with', async () => {
    const fake = new FakeSession().answer(
      'schedule/delete',
      new ApiError({ code: ErrorCode.NO_SCHEDULE, message: 'no such schedule' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    await expect(TestBed.inject(SchedulesService).remove('s1')).rejects.toThrow('no such schedule');
  });

  it('keeps the list read as each change is answered', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [schedule('s1', null), schedule('s2', null)] })
      .answer('schedule/pause', { schedule: { ...schedule('s1', null), paused: true } });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const schedules = TestBed.inject(SchedulesService);
    expect(schedules.schedules()).toBeNull();
    await schedules.refresh();
    await schedules.pause('s1', true);
    await schedules.remove('s2');
    expect(schedules.schedules()?.map((each) => [each.schedule, each.paused])).toEqual([
      ['s1', true],
    ]);
  });

  it('never lets a read a change overtook, or an older read, undo what is newer', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [] })
      .answer('schedule/pause', { schedule: { ...schedule('s1', null), paused: true } });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const schedules = TestBed.inject(SchedulesService);
    await schedules.refresh();
    // A read asked before the pause, answered after it with the schedule as it was.
    const listed = fake.held('schedule/list');
    const slow = schedules.refresh();
    await schedules.pause('s1', true);
    listed({ schedules: [schedule('s1', null)] });
    await slow;
    expect(schedules.schedules()).toEqual([]);
    // Of two reads, the latest is kept, whichever answers first.
    const older = schedules.refresh();
    const giveOlder = listed;
    const newer = fake.held('schedule/list');
    const latest = schedules.refresh();
    newer({ schedules: [{ ...schedule('s1', null), paused: true }] });
    await latest;
    giveOlder({ schedules: [schedule('s1', null)] });
    await older;
    expect(schedules.schedules()?.map((each) => each.paused)).toEqual([true]);
  });

  it('reads again every few seconds while watched, and stops after', async () => {
    vi.useFakeTimers();
    try {
      const fake = new FakeSession().answer('schedule/list', { schedules: [] });
      TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
      const stop = TestBed.inject(SchedulesService).watch();
      expect(fake.paramsOf('schedule/list')).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(TASKS_REFRESH_MS);
      expect(fake.paramsOf('schedule/list')).toHaveLength(2);
      stop();
      await vi.advanceTimersByTimeAsync(TASKS_REFRESH_MS * 3);
      expect(fake.paramsOf('schedule/list')).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });
});
