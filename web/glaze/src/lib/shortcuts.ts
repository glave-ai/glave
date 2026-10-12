/** A parsed keyboard shortcut, such as `mod+k`. */
export interface GlzShortcut {
  readonly key: string;
  /** Cmd on Apple platforms, Ctrl elsewhere. */
  readonly mod: boolean;
  readonly shift: boolean;
  readonly alt: boolean;
}

/** Reads `mod+shift+k`-style text: modifiers `mod`, `shift`, `alt`, then one key. */
export function parseShortcut(text: string): GlzShortcut {
  const parts = text.toLowerCase().split('+');
  const key = parts.at(-1) ?? '';
  return {
    key: key === 'esc' ? 'escape' : key,
    mod: parts.includes('mod'),
    shift: parts.includes('shift'),
    alt: parts.includes('alt'),
  };
}

/** Whether `event` presses `shortcut`; `apple` decides whether `mod` is Cmd or Ctrl. */
export function matchesShortcut(
  event: KeyboardEvent,
  shortcut: GlzShortcut,
  apple: boolean,
): boolean {
  const mod = apple ? event.metaKey : event.ctrlKey;
  const other = apple ? event.ctrlKey : event.metaKey;
  return (
    event.key.toLowerCase() === shortcut.key &&
    mod === shortcut.mod &&
    !other &&
    event.shiftKey === shortcut.shift &&
    event.altKey === shortcut.alt
  );
}

/** Whether this is an Apple platform, where `mod` means Cmd. */
export function isApple(navigator: Navigator): boolean {
  return /mac|iphone|ipad/iu.test(navigator.platform || navigator.userAgent);
}
