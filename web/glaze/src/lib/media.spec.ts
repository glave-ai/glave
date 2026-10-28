import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { fakeMedia } from '../testing/media';
import { mediaMatches } from './media';

/** Something on a page that asks whether the screen is narrow. */
@Component({ template: '', changeDetection: ChangeDetectionStrategy.OnPush })
class NarrowComponent {
  readonly narrow = mediaMatches(() => '(max-width: 48rem)');
}

describe('mediaMatches', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('says whether the query matches at once, and follows the screen as it changes', () => {
    const screen = fakeMedia(true);
    const narrow = TestBed.runInInjectionContext(() => mediaMatches(() => '(max-width: 48rem)'));
    expect(narrow()).toBe(true);
    TestBed.tick();
    screen.set(false);
    expect(narrow()).toBe(false);
    screen.set(true);
    expect(narrow()).toBe(true);
  });

  it('asks again when the query changes', () => {
    const asked: string[] = [];
    vi.stubGlobal('matchMedia', (query: string) => {
      asked.push(query);
      return Object.assign(new EventTarget(), { matches: query === 'b', media: query });
    });
    const query = signal('a');
    const matches = TestBed.runInInjectionContext(() => mediaMatches(query));
    expect(matches()).toBe(false);
    query.set('b');
    expect(matches()).toBe(true);
    expect(asked).toEqual(['a', 'b']);
  });

  it('stops listening once what called it goes', async () => {
    const list = new EventTarget();
    const listening = new Set<unknown>();
    vi.spyOn(list, 'addEventListener').mockImplementation((_, listener) => {
      listening.add(listener);
    });
    vi.spyOn(list, 'removeEventListener').mockImplementation((_, listener) => {
      listening.delete(listener);
    });
    vi.stubGlobal('matchMedia', () => Object.assign(list, { matches: false }));
    const fixture = TestBed.createComponent(NarrowComponent);
    await fixture.whenStable();
    expect(listening.size).toBe(1);
    fixture.destroy();
    expect(listening.size).toBe(0);
  });
});
