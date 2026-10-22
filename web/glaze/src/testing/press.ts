/**
 * Clicks `target` as Safari does, and Firefox on a Mac: the pointer goes down, then the mouse,
 * and unless the page keeps that press (its `mousedown` cancelled) focus leaves what had it and
 * goes nowhere, since those browsers focus no button on a press (jsdom moves no focus at all).
 * The button is held for a task, so what the press started runs before the click, as it does
 * in a browser: a popover's light dismiss, a CDK menu closing once focus has left it.
 */
export async function clickAsSafari(target: HTMLElement): Promise<void> {
  const init = { bubbles: true, cancelable: true, button: 0 };
  target.dispatchEvent(new PointerEvent('pointerdown', init));
  const moved = target.dispatchEvent(new MouseEvent('mousedown', init));
  if (moved && document.activeElement instanceof HTMLElement) {
    document.activeElement.blur();
  }
  await new Promise((resolve) => setTimeout(resolve));
  target.dispatchEvent(new PointerEvent('pointerup', init));
  target.dispatchEvent(new MouseEvent('mouseup', init));
  target.click();
}
