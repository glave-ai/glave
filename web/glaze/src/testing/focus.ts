/**
 * Focuses `target` as the person would: from the keyboard (Tab, from wherever focus was), or
 * with the pointer (a press on it, then the pointer moving off it). jsdom, as browsers do, rings
 * keyboard focus (`:focus-visible`) and not a press's, and takes the pointer to be over what the
 * last mouse event was on (`:hover`), each judged by the input that came last in the page, maybe
 * another test's; so tests say which.
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
