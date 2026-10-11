import { TestBed } from '@angular/core/testing';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { task } from '../testing/fixtures';
import { TasksService } from './tasks.service';

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
});
