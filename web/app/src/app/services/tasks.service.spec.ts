import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { schedule, task } from '../testing/fixtures';
import { TASKS_REFRESH_MS, TasksService } from './tasks.service';

describe('TasksService', () => {
  it('lists tasks, counts those running, and stops one', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', {
        tasks: [
          task('t1', 'Nightly', { status: 'running' }),
          task('t2', 'Elsewhere', { status: 'elsewhere' }),
          task('t3', 'Done', { status: 'finished' }),
        ],
      })
      .answer('schedule/list', { schedules: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const tasks = TestBed.inject(TasksService);
    await tasks.refresh();
    expect(tasks.tasks()).toHaveLength(3);
    expect(tasks.running()).toBe(1);
    expect(tasks.going()).toBe(2);
    expect(tasks.live()).toBe(true);
    await tasks.stop('t1');
    expect(fake.paramsOf('tasks/stop')).toEqual([{ thread: 't1' }]);
    expect(fake.paramsOf('tasks/list')).toHaveLength(2);
  });

  it('means something while a task is going or a schedule has a run to come', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [task('t1', 'Done', { status: 'finished' })] })
      .answer('schedule/list', { schedules: [schedule('s1', null)] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const tasks = TestBed.inject(TasksService);
    await tasks.refresh();
    expect(tasks.live()).toBe(false);
    fake.answer('schedule/list', { schedules: [schedule('s2', '2026-10-12T02:00:00.000Z')] });
    await tasks.refresh();
    expect(tasks.going()).toBe(0);
    expect(tasks.live()).toBe(true);
  });

  it('reads again every few seconds while someone watches, and stops after', async () => {
    vi.useFakeTimers();
    try {
      const fake = new FakeSession()
        .answer('tasks/list', { tasks: [] })
        .answer('schedule/list', { schedules: [] });
      TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
      const tasks = TestBed.inject(TasksService);
      const stopOne = tasks.watch();
      const stopTwo = tasks.watch();
      expect(fake.paramsOf('tasks/list')).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(TASKS_REFRESH_MS);
      expect(fake.paramsOf('tasks/list')).toHaveLength(2);
      stopOne();
      stopOne();
      await vi.advanceTimersByTimeAsync(TASKS_REFRESH_MS);
      expect(fake.paramsOf('tasks/list')).toHaveLength(3);
      stopTwo();
      await vi.advanceTimersByTimeAsync(TASKS_REFRESH_MS * 3);
      expect(fake.paramsOf('tasks/list')).toHaveLength(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it('pauses and removes schedules, keeping each as the server answers', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/list', { schedules: [schedule('s1', null), schedule('s2', null)] })
      .answer('schedule/pause', (params) => {
        const { paused } = params as { readonly paused: boolean };
        return { schedule: { ...schedule('s1', null), paused } };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const tasks = TestBed.inject(TasksService);
    await tasks.refresh();
    const paused = await tasks.pause('s1', true);
    expect(paused.paused).toBe(true);
    expect(tasks.schedules().map((each) => [each.schedule, each.paused])).toEqual([
      ['s1', true],
      ['s2', false],
    ]);
    await tasks.remove('s2');
    expect(fake.paramsOf('schedule/delete')).toEqual([{ schedule: 's2' }]);
    expect(tasks.schedules().map((each) => each.schedule)).toEqual(['s1']);
    // Refused, it says what the server said, and keeps what it had.
    fake.answer('schedule/delete', new ApiError({ code: ErrorCode.NO_SCHEDULE, message: 'gone' }));
    await expect(tasks.remove('s1')).rejects.toThrow('gone');
    expect(tasks.schedules()).toHaveLength(1);
  });

  it('never lets a read a change overtook, or an older read, undo what is newer', async () => {
    const fake = new FakeSession()
      .answer('tasks/list', { tasks: [] })
      .answer('schedule/pause', { schedule: { ...schedule('s1', null), paused: true } });
    const listed = fake.held('schedule/list');
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const tasks = TestBed.inject(TasksService);
    // A read asked before the pause, answered after it with the schedule as it was.
    const slow = tasks.refresh();
    await tasks.pause('s1', true);
    listed({ schedules: [schedule('s1', null)] });
    await slow;
    expect(tasks.schedules()).toEqual([]);
    // Of two reads, the latest is kept, whichever answers first.
    const older = tasks.refresh();
    const giveOlder = listed;
    const newer = fake.held('schedule/list');
    const latest = tasks.refresh();
    newer({ schedules: [{ ...schedule('s1', null), paused: true }] });
    await latest;
    giveOlder({ schedules: [schedule('s1', null)] });
    await older;
    expect(tasks.schedules().map((each) => each.paused)).toEqual([true]);
  });
});
