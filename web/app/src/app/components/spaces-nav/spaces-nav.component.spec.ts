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
    .answer('space/list', { spaces: [space('p1', 'glave'), space('me', 'Personal', 'personal')] })
    .answer('thread/list', (params) => ({
      threads:
        (params as { space: string }).space === 'p1'
          ? [thread('t1', 'Plan the launch'), thread('t2', null, { status: 'waiting' })]
          : [],
    }));
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

function titles(root: HTMLElement): readonly string[] {
  return Array.from(root.querySelectorAll('[data-testid="nav-thread"]')).map((link) =>
    link.textContent.trim(),
  );
}

describe('SpacesNavComponent', () => {
  it('lists each project with its threads, then the loose chats, marking current and waiting', async () => {
    const { root, fixture } = await render();
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const threads = Array.from(root.querySelectorAll<HTMLElement>('[data-testid="nav-thread"]'));
    expect(titles(root)).toEqual(['Plan the launch', 'New chat needs you']);
    expect(threads[0]?.getAttribute('aria-current')).toBe('page');
    expect(
      Array.from(root.querySelectorAll('[data-testid="nav-space-toggle"]')).map((each) =>
        each.textContent.trim(),
      ),
    ).toEqual(['glave', 'Chats']);
    expect(root.textContent).toContain('No threads yet');
    const add = root.querySelectorAll<HTMLAnchorElement>('[data-testid="nav-space-new"]');
    expect(add[0]?.getAttribute('aria-label')).toBe('New thread in glave');
    expect(add[0]?.getAttribute('href')).toBe('/?space=p1');
    expect(add[1]?.getAttribute('href')).toBe('/?space=me');
    await expectAccessible(root);
    threads[0]?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('closes and opens a group from its heading', async () => {
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
    expect(titles(root)).toEqual(['Plan the launch']);
    expect(root.querySelectorAll('[data-testid="nav-space"]')).toHaveLength(1);
    fixture.componentRef.setInput('query', 'zebra');
    await fixture.whenStable();
    expect(titles(root)).toEqual([]);
    expect(root.querySelector('[data-testid="nav-no-match"]')?.textContent).toBe(
      'No threads match',
    );
    await expectAccessible(root);
  });
});
