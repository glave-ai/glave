import {
  DestroyRef,
  Directive,
  ElementRef,
  inject,
  input,
  model,
  numberAttribute,
} from '@angular/core';

/**
 * Keeps a scrolling container at its bottom while its content grows, as a streaming reply
 * does, but only while the person is already there: scrolling up to read stops it, scrolling
 * back down starts it again. `atBottom` tells whether they are there, for a "Jump to latest"
 * button, and `scrollToBottom()` takes them there.
 *
 * ```html
 * <div class="thread" glzStickToBottom #stick="glzStickToBottom" [(atBottom)]="atBottom">…</div>
 * @if (!atBottom()) { <button glzButton (click)="stick.scrollToBottom()">Jump to latest</button> }
 * ```
 */
@Directive({
  selector: '[glzStickToBottom]',
  exportAs: 'glzStickToBottom',
  host: { '(scroll)': 'scrolled()' },
})
export class GlzStickToBottomDirective {
  /** Whether the container is scrolled to its bottom (within `threshold` pixels). */
  readonly atBottom = model(true);
  /** How close to the bottom, in pixels, still counts as at the bottom. */
  readonly threshold = input(24, { transform: numberAttribute });

  private readonly element = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;

  constructor() {
    const view = this.element.ownerDocument.defaultView;
    if (view !== null && typeof view.MutationObserver === 'function') {
      const observer = new view.MutationObserver(() => {
        if (this.atBottom()) {
          this.scrollToBottom();
        }
      });
      observer.observe(this.element, { childList: true, subtree: true, characterData: true });
      inject(DestroyRef).onDestroy(() => {
        observer.disconnect();
      });
    }
  }

  /** Scrolls to the bottom, and sticks there again. */
  scrollToBottom(): void {
    this.element.scrollTop = this.element.scrollHeight;
    this.atBottom.set(true);
  }

  protected scrolled(): void {
    const element = this.element;
    const gap = element.scrollHeight - element.scrollTop - element.clientHeight;
    this.atBottom.set(gap <= this.threshold());
  }
}
