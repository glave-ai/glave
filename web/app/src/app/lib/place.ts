import type { RawEvent } from '@glave/client';

/** What the app knows about where it runs: on this computer or not, and whether a Mac. */
export interface Here {
  /** Whether glave runs on the computer this page is on. */
  readonly local: boolean;
  /** Whether this computer is a Mac. */
  readonly mac: boolean;
}

/**
 * Where a thread's tools act, in plain words, from its latest `workspace.bound`: on the
 * computer glave runs on (this one, a Mac by name, when glave is local; else the server), or on
 * another computer glave made for it. `null` before the thread has a workspace.
 */
export function placeOf(events: readonly RawEvent[], here: Here): string | null {
  const bound = events.filter((event) => event.type === 'workspace.bound').at(-1);
  const computer = bound?.data['computer'];
  const kind =
    typeof computer === 'object' && computer !== null && 'kind' in computer
      ? computer.kind
      : undefined;
  if (kind === 'local') {
    if (!here.local) {
      return $localize`:@@place.server:on the server`;
    }
    return here.mac
      ? $localize`:@@place.mac:on your Mac`
      : $localize`:@@place.computer:on this computer`;
  }
  return kind === 'worker' ? $localize`:@@place.worker:on another computer` : null;
}
