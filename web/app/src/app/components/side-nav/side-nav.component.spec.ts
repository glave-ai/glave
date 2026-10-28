import {
  ComponentFixtureAutoDetect,
  DeferBlockBehavior,
  DeferBlockState,
  TestBed,
} from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { Grouping, Project, TaskInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { GroupingService } from '../../services/grouping.service';
import { ProjectsService } from '../../services/projects.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record, task, thread } from '../../testing/fixtures';
import { SideNavComponent } from './side-nav.component';

/** What the fake server has beside the tasks: the person's projects, and how they group. */
interface Has {
  readonly projects?: readonly Project[];
  readonly grouping?: Grouping;
}

async function render(
  tasks: readonly TaskInfo[] = [],
  { projects = [record('p1', 'glave')], grouping = 'project' }: Has = {},
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<SideNavComponent>>;
  readonly fake: FakeSession;
}> {
  const fake = new FakeSession()
    .answer('project/list', { projects })
    .answer('thread/list', { threads: [thread('t1', 'Plan the launch'), thread('t2', 'Fix CI')] })
    .answer('tasks/list', { tasks })
    .answer('schedule/list', { schedules: [] })
    .answer('grouping/get', { grouping });
  TestBed.configureTestingModule({
    providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(ProjectsService).refresh();
  const fixture = TestBed.createComponent(SideNavComponent);
  fixture.componentRef.setInput('page', 'tasks');
  await fixture.whenStable();
  await settle(fixture);
  return { root: fixture.nativeElement as HTMLElement, fixture, fake };
}

/** Lets what the side nav started settle: its calls, and the code it loads late. */
async function settle(
  fixture: ReturnType<typeof TestBed.createComponent<SideNavComponent>>,
): Promise<void> {
  for (let turn = 0; turn < 3; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
  }
}

describe('SideNavComponent', () => {
  it('keeps the search out of reach until it is named, as it comes in a route’s page', async () => {
    // Nothing binds the view until the test does, as a route's page waits for the next change
    // detection after Angular adds it (Glaze's `INERT_UNTIL_BOUND`).
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(
          new FakeSession()
            .answer('tasks/list', { tasks: [] })
            .answer('schedule/list', { schedules: [] }),
        ),
        { provide: ComponentFixtureAutoDetect, useValue: false },
      ],
    });
    const fixture = TestBed.createComponent(SideNavComponent);
    const root: HTMLElement = fixture.nativeElement;
    const search = (): HTMLInputElement | null =>
      root.querySelector<HTMLInputElement>('input[type="search"]');
    expect(search()?.hasAttribute('aria-label')).toBe(false);
    expect(search()?.hasAttribute('inert')).toBe(true);
    await expectAccessible(root);

    fixture.changeDetectorRef.markForCheck();
    fixture.detectChanges();
    await fixture.whenStable();
    expect(search()?.getAttribute('aria-label')).toBe('Search threads');
    expect(search()?.hasAttribute('inert')).toBe(false);
  });

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

  it('groups the threads by project until the person picks Status in the switch above them', async () => {
    const { root, fixture, fake } = await render();
    const radio = (label: string): HTMLInputElement | undefined =>
      Array.from(root.querySelectorAll<HTMLInputElement>('[data-testid="nav-group"] input')).find(
        (each) => each.labels?.[0]?.textContent.trim() === label,
      );
    expect(radio('Project')?.checked).toBe(true);
    expect(root.querySelector('[data-testid="nav-projects"]')).not.toBeNull();
    expect(root.querySelector('app-status-nav')).toBeNull();
    // Above the threads, as they scroll.
    const group = root.querySelector('app-group-by');
    expect(group?.nextElementSibling?.tagName.toLowerCase()).toBe('app-projects-nav');
    await expectAccessible(root);
    radio('Status')?.click();
    await settle(fixture);
    expect(fake.paramsOf('grouping/set')).toEqual([{ grouping: 'status' }]);
    expect(root.querySelector('[data-testid="nav-projects"]')).toBeNull();
    expect(root.querySelector('[data-testid="nav-idle"]')).not.toBeNull();
    expect(root.querySelectorAll('[data-testid="nav-thread-line"]')).toHaveLength(2);
    await expectAccessible(root);
  });

  it('groups the threads as the person chose in another browser', async () => {
    const { root } = await render([], { grouping: 'status' });
    expect(root.querySelector('app-status-nav')).not.toBeNull();
    expect(root.querySelector('app-projects-nav')).toBeNull();
  });

  it('says so when the threads by status cannot load, and only while they would show', async () => {
    TestBed.configureTestingModule({ deferBlockBehavior: DeferBlockBehavior.Manual });
    const { root, fixture } = await render([], { grouping: 'status' });
    // The switch is the first block, the threads by status the second.
    const status = (await fixture.getDeferBlocks())[1];
    await status?.render(DeferBlockState.Error);
    const unloaded = (): Element | null => root.querySelector('[data-testid="unloaded"]');
    expect(unloaded()?.querySelector('[role="alert"]')?.textContent.trim()).toBe(
      'This part of glave could not load.',
    );
    expect(unloaded()?.querySelector('[data-testid="unloaded-reload"]')?.textContent.trim()).toBe(
      'Reload',
    );
    expect(root.querySelector('app-projects-nav')).toBeNull();
    await expectAccessible(root);
    await TestBed.inject(GroupingService).choose('project');
    await settle(fixture);
    expect(unloaded()).toBeNull();
    expect(root.querySelector('app-projects-nav')).not.toBeNull();
  });

  it('with only Chats, shows no switch and lists the chats alone, whatever was chosen', async () => {
    const { root } = await render([], {
      projects: [record('me', 'Personal', 'personal')],
      grouping: 'status',
    });
    expect(root.querySelector('[data-testid="nav-group"]')).toBeNull();
    expect(root.querySelector('app-status-nav')).toBeNull();
    expect(root.querySelector('[data-testid="nav-chats"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="nav-thread-line"]')).toBeNull();
  });
});
