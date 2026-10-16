import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { SpacesService } from '../../services/spaces.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { space, thread } from '../../testing/fixtures';
import { SideNavComponent } from './side-nav.component';

describe('SideNavComponent', () => {
  it('has New chat, a search, the threads, the pages and the person', async () => {
    const fake = new FakeSession()
      .answer('space/list', { spaces: [space('p1', 'glave')] })
      .answer('thread/list', {
        threads: [thread('t1', 'Plan the launch'), thread('t2', 'Fix CI')],
      });
    TestBed.configureTestingModule({
      providers: [provideRouter([{ path: '**', children: [] }]), ...provideFakeSession(fake)],
    });
    await TestBed.inject(SpacesService).refresh();
    const fixture = TestBed.createComponent(SideNavComponent);
    fixture.componentRef.setInput('page', 'memory');
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="nav-new-chat"]')?.getAttribute('href')).toBe('/');
    expect(root.querySelector('[data-testid="nav-memory"]')?.getAttribute('aria-current')).toBe(
      'page',
    );
    expect(root.querySelector('[data-testid="nav-tasks"]')?.hasAttribute('aria-current')).toBe(
      false,
    );
    expect(root.querySelector('app-person')).not.toBeNull();
    // The theme is chosen in Settings, and browsers install the app from their own menus.
    expect(root.querySelector('[data-testid="theme-picker"], app-install')).toBeNull();
    await expectAccessible(root);

    const search = root.querySelector<HTMLInputElement>('[data-testid="nav-search"]');
    expect(search?.getAttribute('aria-label')).toBe('Search threads');
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
      Array.from(root.querySelectorAll('[data-testid="nav-thread"]')).map((each) =>
        each.textContent.trim(),
      ),
    ).toEqual(['Fix CI']);

    root.querySelector<HTMLElement>('[data-testid="nav-settings"]')?.click();
    root.querySelector<HTMLElement>('[data-testid="nav-thread"]')?.click();
    expect(navigated).toBe(2);
  });
});
