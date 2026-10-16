import { Directive, ElementRef, effect, inject, input } from '@angular/core';

import type { GlzPopoverComponent } from '../components/popover/popover.component';

/**
 * Makes its button open and close a `glz-popover`, natively (the button's popover target), and
 * say so: `aria-expanded` while it is open, `aria-controls` naming it. The popover opens
 * beside this button.
 *
 * ```html
 * <button glzNavItem [glzPopoverTrigger]="mine">You</button>
 * <glz-popover #mine>…</glz-popover>
 * ```
 */
@Directive({
  selector: 'button[glzPopoverTrigger]',
  host: {
    '[attr.aria-expanded]': 'popover().expanded()',
    '[attr.aria-controls]': 'popover().id',
  },
})
export class GlzPopoverTriggerDirective {
  /** The popover this button opens. */
  readonly popover = input.required<GlzPopoverComponent>({ alias: 'glzPopoverTrigger' });

  private readonly host = inject<ElementRef<HTMLButtonElement>>(ElementRef).nativeElement;

  constructor() {
    effect(() => {
      const popover = this.popover();
      popover.attach(this.host);
      this.host.popoverTargetElement = popover.element;
    });
  }
}
