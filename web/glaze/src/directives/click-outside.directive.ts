import { DestroyRef, Directive, ElementRef, inject, output } from '@angular/core';

/**
 * Emits when the pointer goes down anywhere outside its element: for closing something
 * custom that the popover API's light dismiss does not cover.
 *
 * ```html
 * <div class="panel" (glzClickOutside)="close()">…</div>
 * ```
 */
@Directive({ selector: '[glzClickOutside]' })
export class GlzClickOutsideDirective {
  readonly clickOutside = output<PointerEvent>({ alias: 'glzClickOutside' });

  constructor() {
    const element = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    const document = element.ownerDocument;
    const listener = (event: PointerEvent): void => {
      if (event.target instanceof Node && !element.contains(event.target)) {
        this.clickOutside.emit(event);
      }
    };
    document.addEventListener('pointerdown', listener, true);
    inject(DestroyRef).onDestroy(() => {
      document.removeEventListener('pointerdown', listener, true);
    });
  }
}
