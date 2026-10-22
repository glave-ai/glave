import { Directive, ElementRef, inject } from '@angular/core';

/**
 * Focuses its button as it is pressed, in every browser, as Chromium does: the button of a menu
 * that closes once focus leaves it and its button, as the CDK's menus do. Safari, and Firefox
 * on a Mac, focus no button on a press and take focus from what had it, so pressing the button
 * of an open menu closed the menu before the click, and the click opened it again. With focus
 * on the button the press is the menu's own, and the click closes it.
 *
 * ```html
 * <button glzFocusOnPress [cdkMenuTriggerFor]="menu">Model</button>
 * ```
 */
@Directive({
  selector: 'button[glzFocusOnPress]',
  host: { '(mousedown)': 'pressed($event)' },
})
export class GlzFocusOnPressDirective {
  private readonly host = inject<ElementRef<HTMLButtonElement>>(ElementRef).nativeElement;

  /**
   * Takes focus to the button, with no ring, as a press gives it, and keeps the browser from
   * taking it elsewhere after.
   */
  protected pressed(event: MouseEvent): void {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    this.host.focus({ focusVisible: false });
  }
}
