import { TestBed } from '@angular/core/testing';
import { type FakeMedia, fakeMedia } from '@glave/glaze/testing/media';

import { PanelService } from './panel.service';

function stored(): unknown {
  return JSON.parse(localStorage.getItem('glave-thread-panel') ?? 'null');
}

/**
 * The panel's service on a screen whose width the test changes, narrow when `narrow`, once its
 * effects ran, as a page's do before the person can resize it.
 */
function make(narrow = false): {
  readonly panel: PanelService;
  readonly screen: FakeMedia;
} {
  const screen = fakeMedia(narrow);
  const panel = TestBed.inject(PanelService);
  TestBed.tick();
  return { panel, screen };
}

describe('PanelService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('starts shown, and remembers shown or hidden and its width for this browser', () => {
    const { panel } = make();
    expect([panel.showing(), panel.width()]).toEqual([true, null]);
    panel.toggle();
    panel.width.set(420);
    TestBed.tick();
    expect(stored()).toEqual({ open: false, width: 420 });
    TestBed.resetTestingModule();
    const again = make().panel;
    expect([again.showing(), again.width()]).toEqual([false, 420]);
  });

  it('keeps the sheet apart: closed at first, never kept, and the wide choice stands', () => {
    const { panel, screen } = make();
    // Hidden beside a wide page.
    panel.setShowing(false);
    TestBed.tick();
    screen.set(true);
    expect(panel.showing()).toBe(false);
    panel.toggle();
    expect(panel.showing()).toBe(true);
    TestBed.tick();
    expect(stored()).toEqual({ open: false, width: null });
    // Another thread page while narrow: its sheet is closed, the wide choice still hidden.
    TestBed.resetTestingModule();
    const again = make(true);
    expect(again.panel.showing()).toBe(false);
    again.panel.toggle();
    again.screen.set(false);
    TestBed.tick();
    expect(again.panel.showing()).toBe(false);
    expect(stored()).toEqual({ open: false, width: null });
  });

  it('closes the sheet when asked, and leaves the wide choice alone', () => {
    const { panel, screen } = make(true);
    panel.setShowing(true);
    panel.closeSheet();
    expect(panel.showing()).toBe(false);
    screen.set(false);
    expect(panel.showing()).toBe(true);
  });

  it('toggles on its shortcut', () => {
    const { panel } = make();
    const mod = /mac|iphone|ipad/iu.test(navigator.platform || navigator.userAgent)
      ? { metaKey: true }
      : { ctrlKey: true };
    panel.keydown(new KeyboardEvent('keydown', { key: '.', ...mod }));
    expect(panel.showing()).toBe(false);
    panel.keydown(new KeyboardEvent('keydown', { key: '.' }));
    expect(panel.showing()).toBe(false);
    expect(panel.shortcut).toMatch(/^(⌘\.|Ctrl\+\.)$/u);
  });

  it('works without storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const { panel } = make();
    expect(panel.showing()).toBe(true);
    panel.toggle();
    TestBed.tick();
    expect(panel.showing()).toBe(false);
  });
});
