import type { LineKind, RawEvent } from '@glave/client';

import { plural } from './plural';

/** Lines added and removed, as a change shows them (`+12`, `−3`) and as screen readers read them. */
export interface LineCounts {
  readonly added: string;
  readonly removed: string;
  readonly label: string;
}

/** How lines added and removed are marked for `locale`, as a change shows them. */
export interface LineMarks {
  readonly added: (lines: number) => string;
  readonly removed: (lines: number) => string;
}

/** The marks for lines added and removed, written for `locale`: `+12` and `−3`. */
export function lineMarks(locale: string): LineMarks {
  const numbers = new Intl.NumberFormat(locale);
  return {
    added: (lines) => `+${numbers.format(lines)}`,
    removed: (lines) => `−${numbers.format(lines)}`,
  };
}

/** `added` and `removed` lines written for `locale`: the marks a change shows, and in words. */
export function lineCounts(added: number, removed: number, locale: string): LineCounts {
  const marks = lineMarks(locale);
  return {
    added: marks.added(added),
    removed: marks.removed(removed),
    label: plural(
      $localize`:@@files.lines:{${added}:ADDED:, plural, one {# line added} other {# lines added}}, {${removed}:REMOVED:, plural, other {# removed}}`,
      locale,
    ),
  };
}

/** The mark before each line of a change, so colour is never the only sign of what it is. */
export const SIGNS: Readonly<Record<LineKind, string>> = { same: ' ', removed: '-', added: '+' };

/** The tools whose calls can change files, as the server counts them: its own, and a subagent. */
const CHANGING: ReadonlySet<string> = new Set(['write', 'edit', 'agent']);

/**
 * The `seq` of the latest end of a call that can change files (a `write`, an `edit`, or a
 * subagent's `agent` call), or `null` while there is none: when it changes, the thread may have
 * changed files it had not, and its changes are read again. What changed, if anything, is the
 * server's to say.
 */
export function lastChanging(events: readonly RawEvent[]): number | null {
  const asked = new Set<unknown>();
  let last: number | null = null;
  for (const event of events) {
    if (event.type === 'tool.call.requested' && CHANGING.has(String(event.data['tool']))) {
      asked.add(event.data['call_id']);
    } else if (event.type === 'tool.call.completed' && asked.has(event.data['call_id'])) {
      last = event.seq;
    }
  }
  return last;
}
