import type { RawEvent } from '@glave/client';

/** The events that change a thread's plan: the model's or the person's steps, and Build it. */
const CHANGES: ReadonlySet<string> = new Set(['plan.updated', 'plan.approved']);

/**
 * The `seq` of the latest change to the thread's plan, or `null` while it has none: when it
 * changes, the plan is read again. What the plan is, is the server's to say.
 */
export function lastPlanning(events: readonly RawEvent[]): number | null {
  for (let at = events.length - 1; at >= 0; at -= 1) {
    const event = events[at];
    if (event !== undefined && CHANGES.has(event.type)) {
      return event.seq;
    }
  }
  return null;
}
