import { LiveAnnouncer } from '@angular/cdk/a11y';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { ProjectsService } from '../../services/projects.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { schedule, task } from '../../testing/fixtures';
import { TasksComponent } from './tasks.component';

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

async function render(fake: FakeSession): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<TasksComponent>>;
}> {
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake.answer('project/list', { projects: [] })),
      { provide: LiveAnnouncer, useValue: { announce: () => Promise.resolve() } },
    ],
  });
  const fixture = TestBed.createComponent(TasksComponent);
  await settle();
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

/** The page's headings under its own, by their test hooks. */
function headings(root: HTMLElement): readonly (string | null)[] {
  return [...root.querySelectorAll('h2[data-testid]')].map((each) =>
    each.getAttribute('data-testid'),
  );
}

describe('TasksComponent', () => {
  it('shows what runs now and what finished once the side nav’s chats are read, and reads again on Refresh', async () => {
    const fake = new FakeSession().answer('schedule/list', { schedules: [] }).answer('tasks/list', {
      tasks: [
        task('t1', 'Nightly build'),
        task('t2', 'Docs', { status: 'finished', outcome: 'completed' }),
      ],
    });
    const { root, fixture } = await render(fake);
    expect(root.querySelector('h1')?.textContent).toBe('Tasks');
    expect(root.querySelector('[data-testid="page-back"]')).not.toBeNull();
    // Nothing under the heading until the chats it opens with are read: nothing moves after.
    expect(headings(root)).toEqual([]);
    await TestBed.inject(ProjectsService).refresh();
    await settle();
    await fixture.whenStable();
    expect(headings(root)).toEqual(['tasks-running', 'tasks-finished']);
    expect(root.querySelector('[data-testid="tasks-running"]')?.textContent).toBe('Running now');
    expect(root.querySelector('[data-testid="tasks-finished"]')?.textContent).toBe('Finished');
    expect(root.querySelectorAll('[data-testid="task"]')).toHaveLength(2);
    await expectAccessible(root);
    const before = fake.paramsOf('tasks/list').length;
    root.querySelector<HTMLButtonElement>('[data-testid="tasks-refresh"]')?.click();
    await settle();
    expect(fake.paramsOf('tasks/list')).toHaveLength(before + 1);
  });

  it('lists what repeats above what runs now, and no Finished with none finished', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [schedule('s1', null)] })
      .answer('tasks/list', { tasks: [] });
    const { root, fixture } = await render(fake);
    await TestBed.inject(ProjectsService).refresh();
    await settle();
    await fixture.whenStable();
    expect(headings(root)).toEqual(['repeating-heading', 'tasks-running']);
    expect(root.querySelector('[data-testid="task-nothing"]')?.textContent).toBe(
      'Nothing is running.',
    );
    await expectAccessible(root);
  });
});
