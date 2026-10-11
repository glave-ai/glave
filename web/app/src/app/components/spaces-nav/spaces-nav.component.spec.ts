import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { SpacesService } from '../../services/spaces.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { space, thread } from '../../testing/fixtures';
import { SpacesNavComponent } from './spaces-nav.component';

describe('SpacesNavComponent', () => {
  it('lists each space with its threads, marking the current and the waiting', async () => {
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
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const threads = Array.from(root.querySelectorAll('[data-testid="nav-thread"]'));
    expect(threads.map((link) => link.textContent.trim())).toEqual([
      'Plan the launch',
      'New chat needs you',
    ]);
    expect(threads[0]?.getAttribute('aria-current')).toBe('page');
    expect(root.querySelectorAll('[data-testid="nav-space"]')).toHaveLength(2);
    expect(root.textContent).toContain('No threads yet');
    await expectAccessible(root);
    (threads[0] as HTMLElement).click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });
});
