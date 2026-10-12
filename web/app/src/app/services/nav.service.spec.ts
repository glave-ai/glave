import { BreakpointObserver, type BreakpointState } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { BehaviorSubject } from 'rxjs';

import { NavService } from './nav.service';

function stored(): unknown {
  return JSON.parse(localStorage.getItem('glave-side-nav') ?? 'null');
}

describe('NavService', () => {
  let narrow: BehaviorSubject<BreakpointState>;

  beforeEach(() => {
    localStorage.clear();
    narrow = new BehaviorSubject<BreakpointState>({ matches: false, breakpoints: {} });
    TestBed.configureTestingModule({
      providers: [{ provide: BreakpointObserver, useValue: { observe: () => narrow } }],
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows the side nav at first, toggles it, and remembers it on wide screens', () => {
    const nav = TestBed.inject(NavService);
    expect(nav.open()).toBe(true);
    expect(nav.width()).toBeNull();
    nav.toggle();
    nav.width.set(300);
    TestBed.tick();
    expect(stored()).toEqual({ open: false, width: 300 });
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({
      providers: [{ provide: BreakpointObserver, useValue: { observe: () => narrow } }],
    });
    const again = TestBed.inject(NavService);
    expect([again.open(), again.width()]).toEqual([false, 300]);
  });

  it('never stores the drawer of a narrow screen', () => {
    const nav = TestBed.inject(NavService);
    TestBed.tick();
    narrow.next({ matches: true, breakpoints: {} });
    nav.toggle();
    TestBed.tick();
    expect(stored()).toEqual({ open: true, width: null });
    expect(nav.narrow()).toBe(true);
  });

  it('toggles on its shortcut and nothing else', () => {
    const nav = TestBed.inject(NavService);
    const mod = /mac|iphone|ipad/iu.test(navigator.platform || navigator.userAgent)
      ? { metaKey: true }
      : { ctrlKey: true };
    const shortcut = new KeyboardEvent('keydown', { key: 'b', cancelable: true, ...mod });
    nav.keydown(shortcut);
    expect(nav.open()).toBe(false);
    expect(shortcut.defaultPrevented).toBe(true);
    nav.keydown(new KeyboardEvent('keydown', { key: 'b' }));
    expect(nav.open()).toBe(false);
    expect(nav.shortcut).toMatch(/^(⌘B|Ctrl\+B)$/u);
    expect(nav.ariaShortcut).toMatch(/^(Meta|Control)\+B$/u);
  });

  it('works without storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const nav = TestBed.inject(NavService);
    nav.toggle();
    TestBed.tick();
    expect(nav.open()).toBe(false);
  });
});
