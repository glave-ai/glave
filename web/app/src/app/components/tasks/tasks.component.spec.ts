import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { task } from '../../testing/fixtures';
import { TasksComponent } from './tasks.component';

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

describe('TasksComponent', () => {
  it('lists tasks with their cost and time, and stops a running one', async () => {
    const fake = new FakeSession().answer('tasks/list', {
      tasks: [
        task('t1', 'Nightly build'),
        task('t2', 'Docs', { status: 'finished', outcome: 'completed', cost: null }),
      ],
    });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const fixture = TestBed.createComponent(TasksComponent);
    await settle();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const tasks = root.querySelectorAll('[data-testid="task"]');
    expect(tasks).toHaveLength(2);
    expect(tasks[0]?.textContent).toContain('hope · 1.5K tokens · $0.0042 · 2 min 5 s');
    expect(tasks[1]?.textContent).toContain('completed');
    expect(tasks[1]?.textContent).toContain('cost not reported');
    expect(root.querySelectorAll('[data-testid="task-stop"]')).toHaveLength(1);
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="task-stop"]')?.click();
    await settle();
    expect(fake.paramsOf('tasks/stop')).toEqual([{ thread: 't1' }]);
  });

  it('says when there are none', async () => {
    const fake = new FakeSession().answer('tasks/list', { tasks: [] });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const fixture = TestBed.createComponent(TasksComponent);
    await settle();
    await fixture.whenStable();
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('No tasks');
  });
});
