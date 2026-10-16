import { TestBed } from '@angular/core/testing';

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
});
