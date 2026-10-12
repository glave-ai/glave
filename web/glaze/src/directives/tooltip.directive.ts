import { DestroyRef, Directive, ElementRef, effect, inject, input } from '@angular/core';

import { injectId } from '../tokens/ids';
import { type GlzSide, positionPopover } from '../lib/popover';

/**
 * A short hint beside an element, shown on hover and on keyboard focus and hidden by Escape or
 * when the pointer or focus leaves. A native popover (top layer, no z-index games), placed by
 * CSS anchor positioning where the browser has it, else measured. Screen readers get the same
 * text as the element's description, shown or not. For hints only: never the only place
 * something is said.
 *
 * ```html
 * <button glzIconButton label="Copy" [tooltip]="false" glzTooltip="Copy the link">…</button>
 * ```
 */
@Directive({
  selector: '[glzTooltip]',
  host: {
    '(mouseenter)': 'show()',
    '(mouseleave)': 'hide()',
    '(focusin)': 'show()',
    '(focusout)': 'hide()',
  },
})
export class GlzTooltipDirective {
  readonly text = input.required<string>({ alias: 'glzTooltip' });
  readonly position = input<GlzSide>('above', { alias: 'glzTooltipPosition' });

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  private readonly id = injectId('glz-tooltip');
  private readonly panel = this.create();
  private shown = false;
  private readonly escape = (event: KeyboardEvent): void => {
    if (event.key === 'Escape') {
      this.hide();
    }
  };

  constructor() {
    effect(() => {
      this.panel.textContent = this.text();
    });
    const describedBy = this.host.getAttribute('aria-describedby');
    this.host.setAttribute('aria-describedby', describedBy ? `${describedBy} ${this.id}` : this.id);
    inject(DestroyRef).onDestroy(() => {
      this.hide();
      this.panel.remove();
    });
  }

  /** Shows the tooltip. */
  show(): void {
    if (this.shown || this.text() === '') {
      return;
    }
    this.shown = true;
    this.panel.showPopover();
    positionPopover(this.host, this.panel, this.position(), `--${this.id}`);
    this.host.ownerDocument.addEventListener('keydown', this.escape);
  }

  /** Hides the tooltip. */
  hide(): void {
    if (!this.shown) {
      return;
    }
    this.shown = false;
    this.panel.hidePopover();
    this.host.ownerDocument.removeEventListener('keydown', this.escape);
  }

  private create(): HTMLElement {
    const document = this.host.ownerDocument;
    const panel = document.createElement('div');
    panel.id = this.id;
    panel.className = 'glz-tooltip';
    panel.setAttribute('popover', 'manual');
    panel.setAttribute('role', 'tooltip');
    document.body.appendChild(panel);
    return panel;
  }
}
