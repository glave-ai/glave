import { TestBed } from '@angular/core/testing';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { task } from '../testing/fixtures';
import { TASKS_REFRESH_MS, TasksService } from './tasks.service';

describe('TasksService', () => {
  it('lists tasks, counts those running, and stops one', async () => {
    const fake = new FakeSession().answer('tasks/list', {
      tasks: [
        task('t1', 'Nightly', { status: 'running' }),
        task('t2', 'Done', { status: 'finished' }),
      ],
    });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const tasks = TestBed.inject(TasksService);
    await tasks.refresh();
    expect(tasks.tasks()).toHaveLength(2);
    expect(tasks.running()).toBe(1);
    await tasks.stop('t1');
    expect(fake.paramsOf('tasks/stop')).toEqual([{ thread: 't1' }]);
    expect(fake.paramsOf('tasks/list')).toHaveLength(2);
  });

  it('reads again every few seconds while someone watches, and stops after', async () => {
    vi.useFakeTimers();
    try {
      const fake = new FakeSession().answer('tasks/list', { tasks: [] });
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
