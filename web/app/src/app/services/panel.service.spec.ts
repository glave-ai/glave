import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { of } from 'rxjs';

import { PanelService } from './panel.service';

describe('PanelService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('starts shown, and remembers shown or hidden and its width for this browser', () => {
    const panel = TestBed.inject(PanelService);
    expect(panel.open()).toBe(true);
    expect(panel.width()).toBeNull();
    panel.toggle();
    panel.width.set(420);
    TestBed.tick();
    expect(JSON.parse(localStorage.getItem('glave-thread-panel') ?? 'null')).toEqual({
      open: false,
      width: 420,
    });
    TestBed.resetTestingModule();
    const again = TestBed.inject(PanelService);
    expect([again.open(), again.width()]).toEqual([false, 420]);
  });

  it('keeps nothing of the sheet on a narrow screen', () => {
    localStorage.setItem('glave-thread-panel', JSON.stringify({ open: true, width: 400 }));
    TestBed.configureTestingModule({
      providers: [
        { provide: BreakpointObserver, useValue: { observe: () => of({ matches: true }) } },
      ],
    });
    const panel = TestBed.inject(PanelService);
    panel.open.set(false);
    TestBed.tick();
    expect(JSON.parse(localStorage.getItem('glave-thread-panel') ?? 'null')).toEqual({
      open: true,
      width: 400,
    });
  });

  it('toggles on its shortcut', () => {
    const panel = TestBed.inject(PanelService);
    const mod = /mac|iphone|ipad/iu.test(navigator.platform || navigator.userAgent)
      ? { metaKey: true }
      : { ctrlKey: true };
    panel.keydown(new KeyboardEvent('keydown', { key: '.', ...mod }));
    expect(panel.open()).toBe(false);
    panel.keydown(new KeyboardEvent('keydown', { key: '.' }));
    expect(panel.open()).toBe(false);
    expect(panel.shortcut).toMatch(/^(⌘\.|Ctrl\+\.)$/u);
  });

  it('works without storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const panel = TestBed.inject(PanelService);
    expect(panel.open()).toBe(true);
    panel.toggle();
    TestBed.tick();
    expect(panel.open()).toBe(false);
  });
});
