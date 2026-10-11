import { TestBed } from '@angular/core/testing';

import { ThemeService, nextTheme } from './theme.service';

describe('ThemeService', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });

  it('follows the system until the person picks, then sets data-theme and remembers', () => {
    const theme = TestBed.inject(ThemeService);
    expect(theme.choice()).toBe('system');
    theme.set('dark');
    TestBed.tick();
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(localStorage.getItem('glave-theme')).toBe('dark');
    theme.cycle();
    TestBed.tick();
    expect(theme.choice()).toBe('system');
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('reads a remembered choice, and ignores one it does not know', () => {
    localStorage.setItem('glave-theme', 'light');
    expect(TestBed.inject(ThemeService).choice()).toBe('light');
    TestBed.resetTestingModule();
    localStorage.setItem('glave-theme', 'neon');
    expect(TestBed.inject(ThemeService).choice()).toBe('system');
  });

  it('cycles system, light, dark', () => {
    expect(nextTheme('system')).toBe('light');
    expect(nextTheme('light')).toBe('dark');
    expect(nextTheme('dark')).toBe('system');
  });
});
