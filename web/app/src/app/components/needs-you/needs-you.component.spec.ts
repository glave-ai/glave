import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import type { WaitingThread } from '../../lib/nav';
import { NeedsYouComponent } from './needs-you.component';

function waiting(id: string, title: string, place: string | null, current = false): WaitingThread {
  return { id, link: ['/t', id], title, place, current };
}

async function render(threads: readonly WaitingThread[]): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<NeedsYouComponent>>;
}> {
  TestBed.configureTestingModule({ providers: [provideRouter([{ path: '**', children: [] }])] });
  const fixture = TestBed.createComponent(NeedsYouComponent);
  fixture.componentRef.setInput('threads', threads);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

describe('NeedsYouComponent', () => {
  it('lists each thread that needs the person under its heading, with its place', async () => {
    const { root, fixture } = await render([
      waiting('t1', 'Push the branch', 'glave', true),
      waiting('t2', 'Trip to Lisbon', 'Chats'),
    ]);
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    const list = root.querySelector('[data-testid="needs-you"]');
    expect(list?.getAttribute('role')).toBe('group');
    expect(root.querySelector(`#${list?.getAttribute('aria-labelledby') ?? ''}`)?.textContent).toBe(
      'Needs you',
    );
    const threads = Array.from(
      root.querySelectorAll<HTMLAnchorElement>('[data-testid="needs-you-thread"]'),
    );
    expect(threads.map((each) => each.textContent.replace(/\s+/gu, ' ').trim())).toEqual([
      'Push the branch glave',
      'Trip to Lisbon Chats',
    ]);
    expect(threads[0]?.getAttribute('href')).toBe('/t/t1');
    expect(threads[0]?.getAttribute('aria-current')).toBe('page');
    expect(threads[1]?.getAttribute('aria-current')).toBeNull();
    await expectAccessible(root);
    threads[0]?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
  });

  it('names no place when the person has only Chats', async () => {
    const { root } = await render([waiting('t2', 'Trip to Lisbon', null)]);
    expect(root.querySelector('[data-testid="needs-you-thread"]')?.textContent.trim()).toBe(
      'Trip to Lisbon',
    );
    expect(root.querySelector('.place')).toBeNull();
  });

  it('shows nothing when nothing waits', async () => {
    const { root } = await render([]);
    expect(root.querySelector('[data-testid="needs-you"]')).toBeNull();
    expect(getComputedStyle(root).display).toBe('contents');
  });
});
