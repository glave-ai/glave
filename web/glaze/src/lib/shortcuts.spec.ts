import { isApple, isTyping, matchesShortcut, parseShortcut } from './shortcuts';

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

  it('knows when keys are being typed into a field', () => {
    expect(isTyping(document.createElement('textarea'))).toBe(true);
    expect(isTyping(document.createElement('input'))).toBe(true);
    const box = document.createElement('input');
    box.type = 'checkbox';
    expect(isTyping(box)).toBe(false);
    expect(isTyping(document.createElement('button'))).toBe(false);
    expect(isTyping(null)).toBe(false);
  });

  it('recognises Apple platforms', () => {
    expect(isApple({ platform: 'MacIntel', userAgent: '' } as Navigator)).toBe(true);
    expect(isApple({ platform: 'Linux x86_64', userAgent: '' } as Navigator)).toBe(false);
  });
});
