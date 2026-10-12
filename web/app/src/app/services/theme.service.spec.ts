import { TestBed } from '@angular/core/testing';

import { ThemeService } from './theme.service';

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
    expect(localStorage.getItem('glave-theme')).toBe('"dark"');
    theme.set('system');
    TestBed.tick();
    expect(theme.choice()).toBe('system');
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('reads a remembered choice, kept as JSON or as plain text, and ignores one it does not know', () => {
    localStorage.setItem('glave-theme', 'light');
    expect(TestBed.inject(ThemeService).choice()).toBe('light');
    TestBed.resetTestingModule();
    localStorage.setItem('glave-theme', '"dark"');
    expect(TestBed.inject(ThemeService).choice()).toBe('dark');
    TestBed.resetTestingModule();
    localStorage.setItem('glave-theme', 'neon');
    expect(TestBed.inject(ThemeService).choice()).toBe('system');
  });
});
