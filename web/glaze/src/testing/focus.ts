/**
 * Moves focus to `target` as a key or a press would. jsdom judges `:focus-visible` and `:hover`
 * by the last key or mouse event in the window, which may be another test's, so each call brings
 * its own: focus first leaves whatever had it; then a Tab gives the new focus its ring, or a
 * press gives it none and the pointer moves off, so nothing is hovered.
 */
export function focusBy(target: HTMLElement, how: 'keyboard' | 'pointer'): void {
  if (document.activeElement instanceof HTMLElement) {
    document.activeElement.blur();
  }
  if (how === 'keyboard') {
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    target.focus();
  } else {
    target.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
    target.focus();
    target.dispatchEvent(new MouseEvent('mouseout', { bubbles: true }));
  }
}
