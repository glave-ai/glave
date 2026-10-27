import type { PlanInfo, RawEvent } from '@glave/client';

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

/**
 * The plan as the conversation shows it, as a card: a proposal always, since it waits for the
 * person there; a plan being built only while the thread's panel, which carries its progress,
 * is out of sight (`panelShown` false), so the progress is never hidden. `null` for no card.
 */
export function chatPlan(plan: PlanInfo | null, panelShown: boolean): PlanInfo | null {
  return plan !== null && (plan.proposed || !panelShown) ? plan : null;
}
