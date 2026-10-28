import { DOCUMENT } from '@angular/common';
import { type Signal, computed, effect, inject, signal } from '@angular/core';

/**
 * Whether the media query `query` gives matches the page now, following the screen as it
 * changes and the query as it does; `false` where there is no window. Call it where `inject`
 * works: what it listens to goes with what called it.
 */
export function mediaMatches(query: () => string): Signal<boolean> {
  const view = inject(DOCUMENT).defaultView;
  const list = computed(() => view?.matchMedia(query()) ?? null);
  const changes = signal(0);
  effect((onCleanup) => {
    const current = list();
    if (current === null) {
      return;
    }
    const changed = (): void => {
      changes.update((count) => count + 1);
    };
    current.addEventListener('change', changed);
    onCleanup(() => {
      current.removeEventListener('change', changed);
    });
    // Read again, in case the screen changed before anything listened.
    changed();
  });
  return computed(() => {
    changes();
    return list()?.matches ?? false;
  });
}
