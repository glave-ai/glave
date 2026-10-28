import { DestroyRef, Directive, ElementRef, effect, inject, input, untracked } from '@angular/core';

import { countUp } from '../lib/motion';

/**
 * A number as its element's text, written by `glzCountFormat` (the app's, through `Intl` in its
 * locale), in figures of one width. When it grows as the person watches, it counts up from what
 * it showed to the new number over the slow duration, eased out; it shows its first number as
 * it is, a smaller one at once, and every number at once when the person asks for less motion.
 *
 * ```html
 * <span [glzCount]="added()" [glzCountFormat]="plus"></span>
 * ```
 */
@Directive({
  selector: '[glzCount]',
  host: { class: 'glz-count' },
})
export class GlzCountDirective {
  readonly value = input.required<number>({ alias: 'glzCount' });
  /** How a number is written, such as `+1,204`. */
  readonly format = input.required<(value: number) => string>({ alias: 'glzCountFormat' });

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  /** The number shown now, on the way or arrived; `null` before the first. */
  private shown: number | null = null;
  /** Stops the count on its way, if one is. */
  private stop: () => void = () => undefined;

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      this.stop();
    });
    effect(() => {
      const value = this.value();
      const format = this.format();
      untracked(() => {
        this.show(value, format);
      });
    });
  }

  private show(value: number, format: (value: number) => string): void {
    this.stop();
    this.stop = () => undefined;
    const write = (shown: number): void => {
      this.shown = shown;
      this.host.textContent = format(shown);
    };
    const from = this.shown;
    if (from === null || value <= from) {
      write(value);
      return;
    }
    this.stop = countUp(this.host.ownerDocument, from, value, write);
  }
}
