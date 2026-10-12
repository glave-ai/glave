import { TestBed } from '@angular/core/testing';

import { ToolsPanelService } from './tools-panel.service';

describe('ToolsPanelService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('starts closed, and remembers open, width and tab for this browser', () => {
    const panel = TestBed.inject(ToolsPanelService);
    expect(panel.open()).toBe(false);
    expect(panel.width()).toBeNull();
    expect(panel.tabIndex()).toBe(0);
    panel.toggle();
    panel.width.set(420);
    panel.showTab(1);
    panel.showTab(9);
    TestBed.tick();
    expect(JSON.parse(localStorage.getItem('glave-tools-panel') ?? 'null')).toEqual({
      open: true,
      width: 420,
      tab: 'changes',
    });
    TestBed.resetTestingModule();
    const again = TestBed.inject(ToolsPanelService);
    expect([again.open(), again.width(), again.tab()]).toEqual([true, 420, 'changes']);
  });

  it('toggles on its shortcut', () => {
    const panel = TestBed.inject(ToolsPanelService);
    const mod = /mac|iphone|ipad/iu.test(navigator.platform || navigator.userAgent)
      ? { metaKey: true }
      : { ctrlKey: true };
    panel.keydown(new KeyboardEvent('keydown', { key: '.', ...mod }));
    expect(panel.open()).toBe(true);
    panel.keydown(new KeyboardEvent('keydown', { key: '.' }));
    expect(panel.open()).toBe(true);
    expect(panel.shortcut).toMatch(/^(⌘\.|Ctrl\+\.)$/u);
  });

  it('works without storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const panel = TestBed.inject(ToolsPanelService);
    expect(panel.open()).toBe(false);
    panel.toggle();
    TestBed.tick();
    expect(panel.open()).toBe(true);
  });
});
