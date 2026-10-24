import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { ProjectsService } from '../../services/projects.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record, thread } from '../../testing/fixtures';
import { StatusNavComponent } from './status-nav.component';

/** A time on the fixtures' day, at 10:`minute`. */
function at(minute: number): string {
  return `2026-10-11T10:${String(minute).padStart(2, '0')}:00.000Z`;
}

const PROJECTS = [record('p1', 'glave'), record('me', 'Personal', 'personal')];

/** The fake's threads by project: two that wait, one running, and two idle. */
const THREADS: Readonly<Record<string, unknown>> = {
  p1: [
    thread('t1', 'Plan the launch', { updated_at: at(9) }),
    thread('t2', 'Push the branch', { status: 'waiting', updated_at: at(1) }),
    thread('t5', 'Check the parser', { parent: 't1', status: 'running', updated_at: at(8) }),
  ],
  me: [
    thread('t3', 'Trip to Lisbon', { status: 'waiting', updated_at: at(4) }),
    thread('t4', 'Dinner ideas', { updated_at: at(2) }),
  ],
};

interface Rendered {
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<StatusNavComponent>>;
}

async function render(query = ''): Promise<Rendered> {
  const fake = new FakeSession()
    .answer('project/list', { projects: PROJECTS })
    .answer('thread/list', (params) => ({
      threads: THREADS[(params as { project: string }).project] ?? [],
    }));
  TestBed.configureTestingModule({
    providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(ProjectsService).refresh();
  const fixture = TestBed.createComponent(StatusNavComponent);
  fixture.componentRef.setInput('current', 't2');
  fixture.componentRef.setInput('query', query);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function texts(root: ParentNode, testId: string): readonly string[] {
  return Array.from(root.querySelectorAll(`[data-testid="${testId}"]`)).map((each) =>
    each.textContent.replace(/\s+/gu, ' ').trim(),
  );
}

function find(root: ParentNode, testId: string): HTMLElement {
  const found = root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
  if (found === null) {
    throw new Error(`no ${testId}`);
  }
  return found;
}

describe('StatusNavComponent', () => {
  it('lists what needs the person with how many, then what works, then the rest', async () => {
    const { root, fixture } = await render();
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const sections = Array.from(root.querySelectorAll('nav > glz-nav-list'));
    expect(sections.map((each) => each.getAttribute('data-testid'))).toEqual([
      'nav-needs-you',
      'nav-working',
      'nav-idle',
    ]);
    expect(root.querySelector('nav')?.getAttribute('aria-label')).toBe('Threads');
    const heading = (testId: string): string =>
      root.ownerDocument.getElementById(find(root, testId).getAttribute('aria-labelledby') ?? '')
        ?.textContent ?? '';
    expect(['nav-needs-you', 'nav-working', 'nav-idle'].map(heading)).toEqual([
      'Needs you',
      'Working',
      'Idle',
    ]);
    expect(texts(root, 'nav-needs-you-count')).toEqual(['2']);
    // Each newest first, each thread once, with its project or Chats under its title.
    expect(texts(find(root, 'nav-needs-you'), 'nav-thread-title')).toEqual([
      'Trip to Lisbon',
      'Push the branch',
    ]);
    expect(texts(find(root, 'nav-needs-you'), 'nav-thread-line')).toEqual(['Chats', 'glave']);
    expect(texts(find(root, 'nav-working'), 'nav-thread-title')).toEqual(['Check the parser']);
    expect(texts(find(root, 'nav-idle'), 'nav-thread-title')).toEqual([
      'Plan the launch',
      'Dinner ideas',
    ]);
    expect(root.querySelector('[data-testid="nav-children"]')).toBeNull();
    const pushed = Array.from(root.querySelectorAll('[data-testid="nav-thread"]')).find((each) =>
      each.textContent.includes('Push the branch'),
    );
    expect(pushed?.getAttribute('aria-current')).toBe('page');
    await expectAccessible(root);
    find(root, 'nav-thread').click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('keeps the threads a search finds, its sections with none left out, and says when none', async () => {
    const { root, fixture } = await render('lisbon');
    expect(root.querySelectorAll('nav > glz-nav-list')).toHaveLength(1);
    expect(texts(root, 'nav-thread-title')).toEqual(['Trip to Lisbon']);
    expect(texts(root, 'nav-needs-you-count')).toEqual(['1']);
    fixture.componentRef.setInput('query', 'zebra');
    await fixture.whenStable();
    expect(root.querySelectorAll('glz-nav-list')).toHaveLength(0);
    expect(texts(root, 'nav-no-match')).toEqual(['No threads match']);
    await expectAccessible(root);
  });
});
