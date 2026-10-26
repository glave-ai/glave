import type { WorkspaceBound } from '@glave/client';

/** What the app knows about where it runs: on this computer or not, and whether a Mac. */
export interface Here {
  /** Whether glave runs on the computer this page is on. */
  readonly local: boolean;
  /** Whether this computer is a Mac. */
  readonly mac: boolean;
}

/**
 * Where a thread's tools act, in plain words relative to the reader, from where its summary says
 * it works: on the computer glave runs on (this one, a Mac by name, when glave is local; else the
 * server), or on another computer glave made for it. `null` before the thread has a workspace.
 */
export function placeOf(workspace: WorkspaceBound | null, here: Here): string | null {
  switch (workspace?.computer.kind) {
    case 'local':
      if (!here.local) {
        return $localize`:@@place.server:on the server`;
      }
      return here.mac
        ? $localize`:@@place.mac:on your Mac`
        : $localize`:@@place.computer:on this computer`;
    case 'worker':
      return $localize`:@@place.worker:on another computer`;
    case undefined:
      return null;
  }
}
