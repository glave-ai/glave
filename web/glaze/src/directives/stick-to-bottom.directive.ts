import {
  DestroyRef,
  Directive,
  ElementRef,
  inject,
  input,
  model,
  numberAttribute,
  output,
} from '@angular/core';

/** The item the person reads, and how far below the container's top it is, in pixels. */
interface Place {
  readonly item: Element;
  readonly top: number;
}

/**
 * Keeps a scrolling container at its bottom while its content grows, as a streaming reply
 * does, but only while the person is already there: scrolling up to read stops it, scrolling
 * back down starts it again. `atBottom` tells whether they are there, for a "Jump to latest"
 * button, and `scrollToBottom()` takes them there. Away from the bottom, what they read keeps
 * its place while anything above it grows or shrinks, in every browser: the first item in view
 * (a child of the content, found through the elements that hold it alone) stays where it was.
 * `nearTop` says they are within a screen of the top, or the content does not fill the
 * container, so what came before may be read in, above, without moving anything.
 *
 * ```html
 * <div class="thread" glzStickToBottom #stick="glzStickToBottom" [(atBottom)]="atBottom">…</div>
 * @if (!atBottom()) { <button glzButton (click)="stick.scrollToBottom()">Jump to latest</button> }
 * ```
 */
@Directive({
  selector: '[glzStickToBottom]',
  exportAs: 'glzStickToBottom',
  // The browser's own anchoring is off, so it never moves the content twice, nor not at all at
  // the very top, as some do.
  host: { '(scroll)': 'scrolled()', '[style.overflow-anchor]': '"none"' },
})
export class GlzStickToBottomDirective {
  /** Whether the container is scrolled to its bottom (within `threshold` pixels). */
  readonly atBottom = model(true);
  /** How close to the bottom, in pixels, still counts as at the bottom. */
  readonly threshold = input(24, { transform: numberAttribute });
  /** The person is within a screen of the top: after each scroll or change there. */
  readonly nearTop = output();

  private readonly element = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  private place: Place | null = null;

  constructor() {
    const view = this.element.ownerDocument.defaultView;
    if (view === null) {
      return;
    }
    const destroyed = inject(DestroyRef);
    // A change to the content is seen before it is laid out, and one of size after layout but
    // before the next paint, so what is kept in place never shows moved.
    const changed = typeof view.MutationObserver === 'function';
    const sized = typeof view.ResizeObserver === 'function';
    const resizes = sized ? new view.ResizeObserver(() => this.changed()) : null;
    let watched: Element | null = null;
    const changes = changed
      ? new view.MutationObserver(() => {
          this.changed();
          const content = this.content();
          if (resizes !== null && content !== watched) {
            if (watched !== null) {
              resizes.unobserve(watched);
            }
            watched = content;
            resizes.observe(content);
          }
        })
      : null;
    changes?.observe(this.element, { childList: true, subtree: true, characterData: true });
    destroyed.onDestroy(() => {
      changes?.disconnect();
      resizes?.disconnect();
    });
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
    this.look();
  }

  /**
   * The content changed: at the bottom, it is followed; away from it, what the person reads
   * stays put.
   */
  private changed(): void {
    const place = this.place;
    if (this.atBottom()) {
      this.scrollToBottom();
    } else if (place?.item.isConnected === true) {
      const moved = this.offset(place.item) - place.top;
      if (moved !== 0) {
        this.element.scrollTop += moved;
      }
    }
    this.look();
  }

  /**
   * Notes what the person reads now, away from the bottom (at it, nothing is kept in place),
   * and says when they are near the top.
   */
  private look(): void {
    this.place = this.atBottom() ? null : this.inView();
    if (this.element.scrollTop < this.element.clientHeight) {
      this.nearTop.emit();
    }
  }

  /** The element whose children are the items: the container, or what it holds alone. */
  private content(): Element {
    let content: Element = this.element;
    while (content.children.length === 1 && content.firstElementChild !== null) {
      content = content.firstElementChild;
    }
    return content;
  }

  /** The first item at least partly in view, if any. */
  private inView(): Place | null {
    for (const item of Array.from(this.content().children)) {
      const top = this.offset(item);
      if (top + item.getBoundingClientRect().height > 0) {
        return { item, top };
      }
    }
    return null;
  }

  /** How far below the container's top `item` is, in pixels. */
  private offset(item: Element): number {
    return item.getBoundingClientRect().top - this.element.getBoundingClientRect().top;
  }
}
