import {
  Directive,
  ElementRef,
  afterNextRender,
  booleanAttribute,
  inject,
  input,
} from '@angular/core';

/**
 * Moves focus to its element once it has rendered: for the first field of a view that
 * appears in place, where the native `autofocus` attribute (which works on page load and in
 * a `<dialog>`) does not reach. Use it sparingly: moving focus unasked disorients.
 *
 * ```html
 * <textarea glzInput glzAutofocus aria-label="Message"></textarea>
 * ```
 */
@Directive({ selector: '[glzAutofocus]' })
export class GlzAutofocusDirective {
  /** Whether to move focus; bind it to focus only in some cases. */
  readonly enabled = input(true, { alias: 'glzAutofocus', transform: booleanAttribute });

  constructor() {
    const element = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    afterNextRender(() => {
      if (this.enabled()) {
        element.focus();
      }
    });
  }
}
