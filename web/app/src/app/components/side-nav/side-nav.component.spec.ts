import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { TaskInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ProjectsService } from '../../services/projects.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record, task, thread } from '../../testing/fixtures';
import { SideNavComponent } from './side-nav.component';

async function render(tasks: readonly TaskInfo[] = []): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<SideNavComponent>>;
}> {
  const fake = new FakeSession()
    .answer('project/list', { projects: [record('p1', 'glave')] })
    .answer('thread/list', { threads: [thread('t1', 'Plan the launch'), thread('t2', 'Fix CI')] })
    .answer('tasks/list', { tasks })
    .answer('schedule/list', { schedules: [] });
  TestBed.configureTestingModule({
    providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(ProjectsService).refresh();
  const fixture = TestBed.createComponent(SideNavComponent);
  fixture.componentRef.setInput('page', 'tasks');
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

describe('SideNavComponent', () => {
  it('has the name, New chat, a quiet search, the threads and the person', async () => {
    const { root, fixture } = await render();
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    expect(root.querySelector('.name')?.textContent).toBe('glave');
    const newChat = root.querySelector('[data-testid="nav-new-chat"]');
    expect(newChat?.getAttribute('href')).toBe('/');
    expect(newChat?.getAttribute('aria-label')).toBe('New chat');
    expect(root.querySelector('app-person')).not.toBeNull();
    // Nothing about tasks until one is going or scheduled.
    expect(root.querySelector('[data-testid="nav-tasks"]')).toBeNull();
    await expectAccessible(root);

    const search = root.querySelector<HTMLInputElement>('[data-testid="nav-search"]');
    expect(search?.getAttribute('aria-label')).toBe('Search threads');
    expect(search?.placeholder).toBe('Search');
    // Searched as typed: no autocorrect, capitals or spelling.
    expect(search?.getAttribute('autocorrect')).toBe('off');
    expect(search?.getAttribute('autocapitalize')).toBe('off');
    expect(search?.getAttribute('spellcheck')).toBe('false');
    if (search !== null) {
      search.value = 'ci';
      search.dispatchEvent(new Event('input'));
    }
    await fixture.whenStable();
    expect(
      Array.from(root.querySelectorAll('[data-testid="nav-thread-title"]')).map((each) =>
        each.textContent.trim(),
      ),
    ).toEqual(['Fix CI']);
    root.querySelector<HTMLElement>('[data-testid="nav-thread"]')?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('shows Tasks while a task is going, with how many', async () => {
    const { root, fixture } = await render([
      task('t9', 'Nightly audit', { status: 'running' }),
      task('t8', 'Done', { status: 'finished' }),
    ]);
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const tasks = root.querySelector<HTMLElement>('[data-testid="nav-tasks"]');
    expect(tasks?.getAttribute('href')).toBe('/tasks');
    expect(tasks?.getAttribute('aria-current')).toBe('page');
    // The count shows alone; screen readers hear it in words.
    expect(tasks?.querySelector('.going [aria-hidden="true"]')?.textContent).toBe('1');
    expect(tasks?.querySelector('.going .glz-visually-hidden')?.textContent).toBe('1 going');
    await expectAccessible(root);
    tasks?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });
});
