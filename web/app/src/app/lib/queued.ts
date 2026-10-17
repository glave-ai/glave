import type { RawEvent } from '@glave/client';

/**
 * A message the person sent while a turn ran, until the model reads it: `id` is the one the
 * server answered with, the id of the event it is recorded as once read; `null` until then.
 */
export interface Waiting {
  readonly key: string;
  readonly text: string;
  readonly id: string | null;
}

/** `waiting` with the message `key` known by `id`. */
export function named(waiting: readonly Waiting[], key: string, id: string): readonly Waiting[] {
  return waiting.map((message) => (message.key === key ? { ...message, id } : message));
}

/** What in `waiting` the model has not read yet: no event of `events` records it. */
export function notRead(
  waiting: readonly Waiting[],
  events: readonly RawEvent[],
): readonly Waiting[] {
  const recorded = new Set(events.map((event) => event.id));
  return waiting.filter((message) => message.id === null || !recorded.has(message.id));
}

/**
 * Messages the model never read, sent again as one, as Claude Code and Codex send what waited
 * once the person stops a turn: in the order written, a blank line between.
 */
export function together(texts: readonly string[]): string {
  return texts.join('\n\n');
}
