import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import type { NavThread } from '../../lib/nav';
import { NavThreadComponent } from './nav-thread.component';

const SINCE = { at: '2026-10-11T10:05:00.000Z', short: '5m', long: '5 minutes ago' };

function line(id: string, title: string, more: Partial<NavThread> = {}): NavThread {
  return {
    id,
    link: ['/t', id],
    title,
    running: false,
    waiting: false,
    line: null,
    since: SINCE,
    current: false,
    children: [],
    hasChildren: false,
    ...more,
  };
}

async function render(thread: NavThread): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<NavThreadComponent>>;
}> {
  TestBed.configureTestingModule({ providers: [provideRouter([{ path: '**', children: [] }])] });
  const fixture = TestBed.createComponent(NavThreadComponent);
  fixture.componentRef.setInput('thread', thread);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function find(root: ParentNode, testId: string): Element | null {
  return root.querySelector(`[data-testid="${testId}"]`);
}

function text(element: Element | null | undefined): string {
  return element?.textContent.replace(/\s+/gu, ' ').trim() ?? '';
}

describe('NavThreadComponent', () => {
  it('links to the thread with when it was last active, said in words', async () => {
    const { root } = await render(line('t1', 'Plan the launch'));
    const link = find(root, 'nav-thread');
    expect(link?.getAttribute('href')).toBe('/t/t1');
    expect(text(find(root, 'nav-thread-title'))).toBe('Plan the launch');
    expect(link?.getAttribute('aria-current')).toBeNull();
    const since = find(root, 'nav-thread-since');
    expect(since?.getAttribute('datetime')).toBe(SINCE.at);
    expect(text(since?.querySelector('[aria-hidden="true"]'))).toBe('5m');
    expect(text(since?.querySelector('.glz-visually-hidden'))).toBe('5 minutes ago');
    expect(find(root, 'nav-thread-running')).toBeNull();
    // Nothing under its title: it neither waits nor is listed away from its project.
    expect(find(root, 'nav-thread-line')).toBeNull();
    await expectAccessible(root);
  });

  it('turns a ring in its time’s place while it runs, read out as running', async () => {
    const { root } = await render(line('t1', 'Nightly audit', { running: true, current: true }));
    expect(find(root, 'nav-thread')?.getAttribute('aria-current')).toBe('page');
    const ring = find(root, 'nav-thread-running');
    expect(ring?.classList).toContain('glz-spin');
    expect(ring?.getAttribute('role')).toBe('img');
    expect(ring?.getAttribute('aria-label')).toBe('running');
    expect(find(root, 'nav-thread-since')).toBeNull();
    await expectAccessible(root);
  });

  it('says under its title that it waits for the person, bold, its time still at its end', async () => {
    const waits = { text: 'Waiting on you', needsYou: true };
    const { root } = await render(line('t1', 'Fix the build', { waiting: true, line: waits }));
    const said = find(root, 'nav-thread-line');
    expect(text(said)).toBe('Waiting on you');
    expect(said?.classList).toContain('needs-you');
    // Under the title, apart from it in what screen readers hear.
    expect(said?.previousElementSibling).toBe(find(root, 'nav-thread-title'));
    expect(text(find(root, 'nav-thread'))).toMatch(/^Fix the build Waiting on you 5m/u);
    expect(find(root, 'nav-thread-since')).not.toBeNull();
    expect(find(root, 'nav-children')).toBeNull();
    await expectAccessible(root);
  });

  it('says where it is under its title, muted, for a list of threads from every project', async () => {
    const place = { text: 'glave', needsYou: false };
    const { root } = await render(line('t1', 'Fix the build', { line: place }));
    const said = find(root, 'nav-thread-line');
    expect(text(said)).toBe('glave');
    expect(said?.classList).not.toContain('needs-you');
    expect(said?.getAttribute('dir')).toBe('auto');
    await expectAccessible(root);
  });

  it('puts the threads it started under it, and says when any is followed', async () => {
    const child = line('t3', 'Read the logs');
    const started = line('t2', 'Check the parser', { children: [child], hasChildren: true });
    const { root, fixture } = await render(
      line('t1', 'Plan the launch', { children: [started], hasChildren: true }),
    );
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const under = root.querySelector(':scope > [data-testid="nav-children"]');
    expect(
      Array.from(under?.querySelectorAll('[data-testid="nav-thread-title"]') ?? []).map(text),
    ).toEqual(['Check the parser', 'Read the logs']);
    expect(under?.querySelectorAll('[data-testid="nav-children"]')).toHaveLength(1);
    await expectAccessible(root);
    under
      ?.querySelector<HTMLElement>('[data-testid="nav-children"] [data-testid="nav-thread"]')
      ?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });
});
