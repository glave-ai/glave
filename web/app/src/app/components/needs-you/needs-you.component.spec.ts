import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { ProjectsService } from '../../services/projects.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record, thread } from '../../testing/fixtures';
import { NeedsYouComponent } from './needs-you.component';

async function render(waiting: boolean): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<NeedsYouComponent>>;
}> {
  const status = waiting ? 'waiting' : 'idle';
  const fake = new FakeSession()
    .answer('project/list', {
      projects: [record('p1', 'glave'), record('me', 'Personal', 'personal')],
    })
    .answer('thread/list', (params) =>
      (params as { project: string }).project === 'p1'
        ? { threads: [thread('t1', 'Push the branch', { project: 'p1', status })] }
        : { threads: [thread('t2', 'Trip to Lisbon', { project: 'me', status })] },
    );
  TestBed.configureTestingModule({
    providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(ProjectsService).refresh();
  const fixture = TestBed.createComponent(NeedsYouComponent);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

describe('NeedsYouComponent', () => {
  it('counts the threads that need the person and lists each, with its place', async () => {
    const { root, fixture } = await render(true);
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const button = root.querySelector<HTMLButtonElement>('[data-testid="needs-you"]');
    expect(button?.textContent.replace(/\s+/gu, ' ').trim()).toBe('Needs you 2');
    expect(button?.popoverTargetElement).toBe(root.querySelector('[data-testid="needs-you-list"]'));
    const threads = Array.from(
      root.querySelectorAll<HTMLAnchorElement>('[data-testid="needs-you-thread"]'),
    );
    expect(threads.map((each) => each.textContent.replace(/\s+/gu, ' ').trim())).toEqual([
      'Push the branch glave',
      'Trip to Lisbon Chats',
    ]);
    expect(threads[0]?.getAttribute('href')).toBe('/t/t1');
    await expectAccessible(root);
    threads[0]?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('shows no count, and says so, when nothing needs the person', async () => {
    const { root } = await render(false);
    expect(root.querySelector('[data-testid="needs-you-count"]')).toBeNull();
    expect(root.querySelector('[data-testid="needs-you-none"]')?.textContent).toBe(
      'Nothing needs you right now.',
    );
    await expectAccessible(root);
  });
});
