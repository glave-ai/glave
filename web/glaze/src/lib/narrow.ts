import { type Signal, type WritableSignal, effect, untracked } from '@angular/core';

/**
 * Closes what slides over the page on a narrow screen (a drawer, a sheet) whenever the screen
 * becomes narrow, and puts it back as the person left it once the screen is wide again. Called
 * where `effect` works, as a component is made.
 */
export function closedWhileNarrow(narrow: Signal<boolean>, open: WritableSignal<boolean>): void {
  let wideOpen: boolean | null = null;
  effect(() => {
    const now = narrow();
    untracked(() => {
      if (now && wideOpen === null) {
        wideOpen = open();
        open.set(false);
      } else if (!now && wideOpen !== null) {
        open.set(wideOpen);
        wideOpen = null;
      }
    });
  });
}
