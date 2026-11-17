import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { TASKS_REFRESH_MS, TasksService } from '../../services/tasks.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { task } from '../../testing/fixtures';
import { TaskListComponent } from './task-list.component';

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

function make(
  fake: FakeSession,
  finished = false,
): ReturnType<typeof TestBed.createComponent<TaskListComponent>> {
  TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
  const fixture = TestBed.createComponent(TaskListComponent);
  fixture.componentRef.setInput('finished', finished);
  return fixture;
}

const TASKS = [
  task('t1', 'Nightly build'),
  task('t2', 'Docs', { status: 'finished', outcome: 'completed', cost: null }),
  task('t3', 'Elsewhere', { status: 'elsewhere' }),
  task('t4', 'Audit', { status: 'finished', outcome: 'limit_reached' }),
];

describe('TaskListComponent', () => {
  it('lists the tasks going with their cost and time, and stops a running one', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [] })
      .answer('tasks/list', { tasks: TASKS });
    const fixture = make(fake);
    await settle();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const tasks = root.querySelectorAll('[data-testid="task"]');
    expect(tasks).toHaveLength(2);
    expect(tasks[0]?.textContent).toContain('hope · 1.5K tokens · $0.0042 · 2m 05s');
    expect(tasks[0]?.querySelector('a')?.getAttribute('href')).toBe('/t/t1');
    expect(tasks[1]?.textContent).toContain('running elsewhere');
    expect(root.querySelector('[data-testid="task-nothing"]')).toBeNull();
    expect(root.querySelectorAll('[data-testid="task-stop"]')).toHaveLength(1);
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="task-stop"]')?.click();
    await settle();
    expect(fake.paramsOf('tasks/stop')).toEqual([{ thread: 't1' }]);
  });

  it('lists the tasks finished, each with how it ended', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [] })
      .answer('tasks/list', { tasks: TASKS });
    const fixture = make(fake, true);
    await settle();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const tasks = root.querySelectorAll('[data-testid="task"]');
    expect(tasks).toHaveLength(2);
    expect(tasks[0]?.textContent).toContain('completed');
    expect(tasks[0]?.textContent).toContain('cost not reported');
    expect(tasks[1]?.textContent).toContain('stopped at its limit');
    expect(root.querySelectorAll('[data-testid="task-stop"]')).toHaveLength(0);
    await expectAccessible(root);
  });

  it('says when nothing is running, and what went wrong', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [] })
      .answer('tasks/list', { tasks: [] });
    const fixture = make(fake);
    await settle();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="task-nothing"]')?.textContent).toBe(
      'Nothing is running.',
    );
    fake.answer('tasks/list', new ApiError({ code: ErrorCode.INTERNAL, message: 'down' }));
    await TestBed.inject(TasksService).refresh();
    await fixture.whenStable();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain('down');
  });

  it('reads the list again while it shows, and stops when it goes', async () => {
    vi.useFakeTimers();
    try {
      const fake = new FakeSession()
        .answer('schedule/list', { schedules: [] })
        .answer('tasks/list', { tasks: [] });
      const fixture = make(fake);
      await vi.advanceTimersByTimeAsync(TASKS_REFRESH_MS);
      expect(fake.paramsOf('tasks/list')).toHaveLength(2);
      fixture.destroy();
      await vi.advanceTimersByTimeAsync(TASKS_REFRESH_MS * 2);
      expect(fake.paramsOf('tasks/list')).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });
});
