// Unit tests run in jsdom, which has no media queries. This gives a test a screen whose every
// query matches or not, as it says, and tells each query's listeners when that changes.

/** A screen a test changes. */
export interface FakeMedia {
  /** Makes every query match, or none, and tells their listeners. */
  readonly set: (matches: boolean) => void;
}

/**
 * Puts a fake `matchMedia` on the window, whose every query matches when `matches`. The test
 * takes it away with `vi.unstubAllGlobals()`.
 */
export function fakeMedia(matches = false): FakeMedia {
  let now = matches;
  const lists: EventTarget[] = [];
  class FakeList extends EventTarget implements MediaQueryList {
    onchange: MediaQueryList['onchange'] = null;
    constructor(readonly media: string) {
      super();
    }
    get matches(): boolean {
      return now;
    }
    addListener(): void {
      // Deprecated; Glaze listens with `addEventListener`.
    }
    removeListener(): void {
      // Deprecated; Glaze listens with `addEventListener`.
    }
  }
  vi.stubGlobal('matchMedia', (query: string): MediaQueryList => {
    const list = new FakeList(query);
    lists.push(list);
    return list;
  });
  return {
    set: (next) => {
      now = next;
      for (const list of lists) {
        list.dispatchEvent(new Event('change'));
      }
    },
  };
}
