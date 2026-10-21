import { ApiError, ErrorCode, type RawEvent } from '@glave/client';

import { sentence, somethingWrong } from './text';

/** What a thread records as one of its commands in the background starts or ends. */
const COMMAND_EVENTS: ReadonlySet<string> = new Set(['command.started', 'command.ended']);

/**
 * The `seq` of the latest start or end of a command in the background that `events` record, or
 * `null` when they record none: when it changes, what runs in the background has changed.
 */
export function lastCommandChange(events: readonly RawEvent[]): number | null {
  return events.reduce<number | null>(
    (last, event) => (COMMAND_EVENTS.has(event.type) ? event.seq : last),
    null,
  );
}

/**
 * How long a command started at `startedAt` has run at `now` (milliseconds since the epoch), in
 * whole seconds, as milliseconds: at least a second, so a clock never reads less. `null` when
 * its start is not recorded yet or does not parse.
 */
export function runningFor(startedAt: string | null | undefined, now: number): number | null {
  const start = typeof startedAt === 'string' ? Date.parse(startedAt) : Number.NaN;
  if (Number.isNaN(start)) {
    return null;
  }
  return Math.max(1, Math.floor((now - start) / 1000)) * 1000;
}

/** Whether `error` says the command is not running: it ended before `command/stop` reached it. */
export function alreadyEnded(error: unknown): boolean {
  return error instanceof ApiError && error.code === ErrorCode.NO_COMMAND;
}

/**
 * What to tell the person when the commands could not be read or one could not be stopped: the
 * server's own words when it refused, else the plain failure.
 */
export function commandProblem(error: unknown): string {
  return error instanceof ApiError ? sentence(error.message) : somethingWrong();
}
