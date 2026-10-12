import { isApple, matchesShortcut, parseShortcut } from './shortcuts';

function press(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  return new KeyboardEvent('keydown', { key, ...init });
}

describe('shortcuts', () => {
  it('parses modifiers and the key', () => {
    expect(parseShortcut('mod+Shift+K')).toEqual({
      key: 'k',
      mod: true,
      shift: true,
      alt: false,
    });
    expect(parseShortcut('esc').key).toBe('escape');
  });

  it('matches mod as Cmd on Apple platforms and Ctrl elsewhere, exactly', () => {
    const modK = parseShortcut('mod+k');
    expect(matchesShortcut(press('k', { metaKey: true }), modK, true)).toBe(true);
    expect(matchesShortcut(press('k', { ctrlKey: true }), modK, true)).toBe(false);
    expect(matchesShortcut(press('k', { ctrlKey: true }), modK, false)).toBe(true);
    expect(matchesShortcut(press('k', { ctrlKey: true, shiftKey: true }), modK, false)).toBe(false);
    expect(matchesShortcut(press('j', { ctrlKey: true }), modK, false)).toBe(false);
  });

  it('recognises Apple platforms', () => {
    expect(isApple({ platform: 'MacIntel', userAgent: '' } as Navigator)).toBe(true);
    expect(isApple({ platform: 'Linux x86_64', userAgent: '' } as Navigator)).toBe(false);
  });
});
