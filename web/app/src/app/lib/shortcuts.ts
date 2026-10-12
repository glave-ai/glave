import { type GlzShortcut, isApple, matchesShortcut, parseShortcut } from '@glave/glaze';

/** The app's keyboard shortcuts, in Glaze's form: `mod` is Cmd on Apple platforms, else Ctrl. */
export const SHORTCUTS = {
  /** Shows or hides the side nav. */
  nav: 'mod+b',
  /** Shows or hides a thread's tools panel. */
  tools: 'mod+.',
} as const;

/** Whether the page runs on an Apple platform, where `mod` is Cmd. */
export function onApple(document: Document): boolean {
  const navigator = document.defaultView?.navigator;
  return navigator === undefined ? false : isApple(navigator);
}

function keyName(shortcut: GlzShortcut): string {
  return shortcut.key.length === 1 ? shortcut.key.toUpperCase() : shortcut.key;
}

/** A shortcut as the person reads it: `⌘B` on Apple platforms, `Ctrl+B` elsewhere. */
export function shortcutLabel(keys: string, apple: boolean): string {
  const shortcut = parseShortcut(keys);
  if (apple) {
    const marks = [shortcut.alt ? '⌥' : '', shortcut.shift ? '⇧' : '', shortcut.mod ? '⌘' : ''];
    return `${marks.join('')}${keyName(shortcut)}`;
  }
  const parts = [
    shortcut.mod ? 'Ctrl' : '',
    shortcut.alt ? 'Alt' : '',
    shortcut.shift ? 'Shift' : '',
  ];
  return [...parts.filter((part) => part !== ''), keyName(shortcut)].join('+');
}

/** A shortcut in the form `aria-keyshortcuts` takes: `Meta+B`, or `Control+B`. */
export function ariaShortcut(keys: string, apple: boolean): string {
  const shortcut = parseShortcut(keys);
  const parts = [
    shortcut.mod ? (apple ? 'Meta' : 'Control') : '',
    shortcut.alt ? 'Alt' : '',
    shortcut.shift ? 'Shift' : '',
  ];
  return [...parts.filter((part) => part !== ''), keyName(shortcut)].join('+');
}

/** Whether `event` presses the shortcut `keys`. */
export function pressed(event: KeyboardEvent, keys: string, apple: boolean): boolean {
  return matchesShortcut(event, parseShortcut(keys), apple);
}
