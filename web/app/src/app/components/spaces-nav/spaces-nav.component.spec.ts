import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { SpacesService } from '../../services/spaces.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { space, thread } from '../../testing/fixtures';
import { SpacesNavComponent } from './spaces-nav.component';

async function render(query = ''): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<SpacesNavComponent>>;
}> {
  const fake = new FakeSession()
    .answer('space/list', {
      spaces: [space('p1', 'glave'), space('p2', 'Garden'), space('me', 'Personal', 'personal')],
    })
    .answer('thread/list', (params) => {
      const threads: Readonly<Record<string, unknown>> = {
        p1: [thread('t1', 'Plan the launch'), thread('t2', null, { status: 'waiting' })],
        me: [thread('t3', 'Trip to Lisbon', { status: 'running' })],
      };
      return { threads: threads[(params as { space: string }).space] ?? [] };
    });
  TestBed.configureTestingModule({
    providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(SpacesService).refresh();
  const fixture = TestBed.createComponent(SpacesNavComponent);
  fixture.componentRef.setInput('current', 't1');
  fixture.componentRef.setInput('query', query);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function texts(root: Element, testId: string): readonly string[] {
  return Array.from(root.querySelectorAll(`[data-testid="${testId}"]`)).map((each) =>
    each.textContent.replace(/\s+/gu, ' ').trim(),
  );
}

describe('SpacesNavComponent', () => {
  it('lists the chats, then each project with its threads under it, marked quietly', async () => {
    const { root, fixture } = await render();
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const chats = root.querySelector('[data-testid="nav-chats"]');
    const projects = root.querySelector('[data-testid="nav-projects"]');
    expect(chats?.querySelector('.heading')?.textContent).toBe('Chats');
    expect(projects?.querySelector('.heading')?.textContent).toBe('Projects');
    expect(chats ? texts(chats, 'nav-thread') : []).toEqual(['Trip to Lisbon running']);
    expect(texts(root, 'nav-space-toggle')).toEqual(['glave', 'Garden']);
    expect(texts(root, 'nav-thread')).toEqual([
      'Trip to Lisbon running',
      'Plan the launch',
      'New chat needs you',
    ]);
    const threads = Array.from(root.querySelectorAll<HTMLElement>('[data-testid="nav-thread"]'));
    expect(threads[1]?.getAttribute('aria-current')).toBe('page');
    // Quiet marks: the word alone for running, a dot for what needs the person.
    expect(threads[0]?.querySelector('glz-badge')?.classList).toContain('glz-badge--plain');
    expect(threads[2]?.querySelector('glz-badge')?.classList).toContain('glz-badge--dot');
    // An empty project shows no filler.
    expect(root.textContent).not.toContain('No threads');
    const add = root.querySelectorAll<HTMLAnchorElement>('[data-testid="nav-space-new"]');
    expect(add[0]?.getAttribute('aria-label')).toBe('New thread in glave');
    expect(add[0]?.getAttribute('href')).toBe('/?space=p1');
    expect(add[1]?.getAttribute('href')).toBe('/?space=p2');
    await expectAccessible(root);
    threads[1]?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('closes and opens a project from its row', async () => {
    const { root, fixture } = await render();
    const toggle = root.querySelector<HTMLButtonElement>('[data-testid="nav-space-toggle"]');
    toggle?.click();
    await fixture.whenStable();
    expect(toggle?.getAttribute('aria-expanded')).toBe('false');
    expect(root.querySelector<HTMLElement>('[data-testid="nav-space-items"]')?.hidden).toBe(true);
    toggle?.click();
    await fixture.whenStable();
    expect(toggle?.getAttribute('aria-expanded')).toBe('true');
  });

  it('keeps the threads whose title matches the search, and says when none do', async () => {
    const { root, fixture } = await render('launch');
    expect(texts(root, 'nav-thread')).toEqual(['Plan the launch']);
    expect(root.querySelector('[data-testid="nav-chats"]')).toBeNull();
    expect(root.querySelectorAll('[data-testid="nav-space"]')).toHaveLength(1);
    fixture.componentRef.setInput('query', 'zebra');
    await fixture.whenStable();
    expect(texts(root, 'nav-thread')).toEqual([]);
    expect(root.querySelector('[data-testid="nav-no-match"]')?.textContent).toBe(
      'No threads match',
    );
    await expectAccessible(root);
  });
});
