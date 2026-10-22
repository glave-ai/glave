import { type Signal, effect, signal } from '@angular/core';

/** How often a clock moves on, in milliseconds. */
export const TICK_MS = 1_000;

/**
 * Now, in milliseconds since the epoch, for a time that counts up as the person watches: moving
 * on each second while `running()` holds, and resting otherwise. Its timer goes with what made
 * it. Call it where `inject` may be called, as a field is made.
 */
export function clock(running: () => boolean): Signal<number> {
  const now = signal(Date.now());
  effect((onCleanup) => {
    if (!running()) {
      return;
    }
    now.set(Date.now());
    const timer = setInterval(() => {
      now.set(Date.now());
    }, TICK_MS);
    onCleanup(() => {
      clearInterval(timer);
    });
  });
  return now.asReadonly();
}
