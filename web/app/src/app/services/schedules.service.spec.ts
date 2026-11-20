import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { schedule } from '../testing/fixtures';
import { SchedulesService } from './schedules.service';
import { TASKS_REFRESH_MS } from './tasks.service';

describe('SchedulesService', () => {
  it('pauses, resumes and removes a schedule, answering as the server has it', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [] })
      .answer('schedule/pause', (params) => {
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

  it('keeps each change as the server answers it, then reads the list again', async () => {
    let listed = [schedule('s1', null), schedule('s2', null)];
    const fake = new FakeSession()
      .answer('schedule/list', () => ({ schedules: listed }))
      .answer('schedule/pause', { schedule: { ...schedule('s1', null), paused: true } });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const schedules = TestBed.inject(SchedulesService);
    expect([schedules.schedules(), schedules.read()]).toEqual([null, false]);
    await schedules.refresh();
    expect(schedules.read()).toBe(true);
    listed = [{ ...schedule('s1', null), paused: true }];
    await schedules.pause('s1', true);
    await schedules.remove('s2');
    expect(schedules.schedules()?.map((each) => [each.schedule, each.paused])).toEqual([
      ['s1', true],
    ]);
    // Each change reads the list again.
    expect(fake.paramsOf('schedule/list')).toHaveLength(3);
  });

  it('never lets a read asked before a change undo it: the latest read wins', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [schedule('s1', null)] })
      .answer('schedule/pause', { schedule: { ...schedule('s1', null), paused: true } });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const schedules = TestBed.inject(SchedulesService);
    await schedules.refresh();
    // A read asked before the pause, answered after it with the schedule as it was.
    const before = fake.held('schedule/list');
    const slow = schedules.refresh();
    const after = fake.held('schedule/list');
    await schedules.pause('s1', true);
    after({ schedules: [{ ...schedule('s1', null), paused: true }] });
    await new Promise((resolve) => setTimeout(resolve));
    before({ schedules: [schedule('s1', null)] });
    await slow;
    expect(schedules.schedules()?.map((each) => each.paused)).toEqual([true]);
  });

  it('counts a list it could not read as read, keeping what it had', async () => {
    const fake = new FakeSession().answer(
      'schedule/list',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'down' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const schedules = TestBed.inject(SchedulesService);
    await schedules.refresh();
    expect([schedules.schedules(), schedules.read()]).toEqual([null, true]);
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
