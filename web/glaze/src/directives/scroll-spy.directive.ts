import {
  DestroyRef,
  Directive,
  ElementRef,
  afterNextRender,
  inject,
  input,
  model,
} from '@angular/core';

/**
 * Tells which section of a scrolling container is in view, for a table of contents that
 * follows the reader. Watches its elements matching `targets` (by default, those
 * with an id) with an IntersectionObserver; `active` is the id of the first one in view.
 *
 * ```html
 * <main glzScrollSpy [(active)]="section">
 *   <section id="forms">…</section>
 *   <section id="overlays">…</section>
 * </main>
 * ```
 */
@Directive({ selector: '[glzScrollSpy]' })
export class GlzScrollSpyDirective {
  /** The id of the first watched section in view. */
  readonly active = model<string | null>(null);
  /** Which elements to watch, as a selector. */
  readonly targets = input('[id]');

  constructor() {
    const element = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    const destroyRef = inject(DestroyRef);
    afterNextRender(() => {
      const view = element.ownerDocument.defaultView;
      if (view === null || typeof view.IntersectionObserver !== 'function') {
        return;
      }
      const sections = Array.from(element.querySelectorAll<HTMLElement>(this.targets()));
      const visible = new Set<Element>();
      const observer = new view.IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            if (entry.isIntersecting) {
              visible.add(entry.target);
            } else {
              visible.delete(entry.target);
            }
          }
          const first = sections.find((section) => visible.has(section));
          if (first !== undefined) {
            this.active.set(first.id);
          }
        },
        { rootMargin: '0px 0px -60% 0px' },
      );
      for (const section of sections) {
        observer.observe(section);
      }
      destroyRef.onDestroy(() => {
        observer.disconnect();
      });
    });
  }
}
