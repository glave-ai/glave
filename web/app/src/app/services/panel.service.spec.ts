import { BreakpointObserver, type BreakpointState } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { BehaviorSubject } from 'rxjs';

import { PanelService } from './panel.service';

function stored(): unknown {
  return JSON.parse(localStorage.getItem('glave-thread-panel') ?? 'null');
}

/** The panel's service on a screen whose width the test changes, narrow when `narrow`. */
function make(narrow = false): {
  readonly panel: PanelService;
  readonly screen: BehaviorSubject<BreakpointState>;
} {
  const screen = new BehaviorSubject<BreakpointState>({ matches: narrow, breakpoints: {} });
  TestBed.configureTestingModule({
    providers: [{ provide: BreakpointObserver, useValue: { observe: () => screen } }],
  });
  return { panel: TestBed.inject(PanelService), screen };
}

describe('PanelService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
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
    screen.next({ matches: true, breakpoints: {} });
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
    again.screen.next({ matches: false, breakpoints: {} });
    TestBed.tick();
    expect(again.panel.showing()).toBe(false);
    expect(stored()).toEqual({ open: false, width: null });
  });

  it('closes the sheet when asked, and leaves the wide choice alone', () => {
    const { panel, screen } = make(true);
    panel.setShowing(true);
    panel.closeSheet();
    expect(panel.showing()).toBe(false);
    screen.next({ matches: false, breakpoints: {} });
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
