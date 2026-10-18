import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import type { NavThread } from '../../lib/nav';
import { threadMark } from '../../lib/threads';
import { NavThreadComponent } from './nav-thread.component';

function line(id: string, title: string, more: Partial<NavThread> = {}): NavThread {
  return {
    id,
    link: ['/t', id],
    title,
    mark: null,
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

describe('NavThreadComponent', () => {
  it('links to the thread, marked quietly, current when it is shown', async () => {
    const { root } = await render(
      line('t1', 'Fix the build', { current: true, mark: threadMark({ status: 'waiting' }) }),
    );
    const link = root.querySelector('[data-testid="nav-thread"]');
    expect(link?.getAttribute('href')).toBe('/t/t1');
    expect(link?.getAttribute('aria-current')).toBe('page');
    expect(link?.textContent.replace(/\s+/gu, ' ').trim()).toBe('Fix the build needs you');
    expect(link?.querySelector('glz-badge')?.classList).toContain('glz-badge--dot');
    expect(root.querySelector('[data-testid="nav-children"]')).toBeNull();
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
      Array.from(under?.querySelectorAll('[data-testid="nav-thread"]') ?? []).map((each) =>
        each.textContent.trim(),
      ),
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
