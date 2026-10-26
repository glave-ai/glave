import { isApple } from '@glave/glaze';

import { SHORTCUTS, ariaShortcut, onApple, pressed, shortcutLabel } from './shortcuts';

describe('shortcuts', () => {
  it('reads as ⌘ on Apple platforms and Ctrl elsewhere', () => {
    expect(shortcutLabel(SHORTCUTS.nav, true)).toBe('⌘B');
    expect(shortcutLabel(SHORTCUTS.panel, true)).toBe('⌘.');
    expect(shortcutLabel(SHORTCUTS.nav, false)).toBe('Ctrl+B');
    expect(shortcutLabel('mod+shift+alt+k', true)).toBe('⌥⇧⌘K');
    expect(shortcutLabel('mod+shift+alt+k', false)).toBe('Ctrl+Alt+Shift+K');
  });

  it('names the keys for aria-keyshortcuts', () => {
    expect(ariaShortcut(SHORTCUTS.nav, true)).toBe('Meta+B');
    expect(ariaShortcut(SHORTCUTS.panel, false)).toBe('Control+.');
    expect(ariaShortcut('shift+alt+escape', false)).toBe('Alt+Shift+escape');
  });

  it('matches a key press, with Cmd on Apple platforms and Ctrl elsewhere', () => {
    const cmdB = new KeyboardEvent('keydown', { key: 'b', metaKey: true });
    const ctrlB = new KeyboardEvent('keydown', { key: 'b', ctrlKey: true });
    expect(pressed(cmdB, SHORTCUTS.nav, true)).toBe(true);
    expect(pressed(cmdB, SHORTCUTS.nav, false)).toBe(false);
    expect(pressed(ctrlB, SHORTCUTS.nav, false)).toBe(true);
    expect(pressed(ctrlB, SHORTCUTS.panel, false)).toBe(false);
  });

  it('knows the platform from the page', () => {
    expect(onApple(document)).toBe(isApple(navigator));
  });
});
