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
  it('shows the tasks under the page’s heading, and reads them again on Refresh', async () => {
    const fake = new FakeSession()
      .answer('schedule/list', { schedules: [] })
      .answer('tasks/list', { tasks: [task('t1', 'Nightly build')] });
    TestBed.configureTestingModule({ providers: [provideRouter([]), ...provideFakeSession(fake)] });
    const fixture = TestBed.createComponent(TasksComponent);
    await settle();
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('h1')?.textContent).toBe('Tasks');
    expect(root.querySelector('[data-testid="page-back"]')).not.toBeNull();
    expect(root.querySelectorAll('[data-testid="task"]')).toHaveLength(1);
    await expectAccessible(root);
    const before = fake.paramsOf('tasks/list').length;
    root.querySelector<HTMLButtonElement>('[data-testid="tasks-refresh"]')?.click();
    await settle();
    expect(fake.paramsOf('tasks/list')).toHaveLength(before + 1);
  });
});
