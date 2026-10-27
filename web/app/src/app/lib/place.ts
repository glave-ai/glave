import type { WorkspaceBound } from '@glave/client';

/** What the app knows about where it runs: on this computer or not, and whether a Mac. */
export interface Here {
  /** Whether glave runs on the computer this page is on. */
  readonly local: boolean;
  /** Whether this computer is a Mac. */
  readonly mac: boolean;
}

/**
 * The computer a thread's tools act on, named relative to the reader, from where its summary
 * says it works: the computer glave runs on (this one, a Mac by name, when glave is local; else
 * the server), or another computer glave made for it. `null` before the thread has a workspace.
 */
export function placeOf(workspace: WorkspaceBound | null, here: Here): string | null {
  switch (workspace?.computer.kind) {
    case 'local':
      if (!here.local) {
        return $localize`:@@place.server:The server`;
      }
      return here.mac
        ? $localize`:@@place.mac:Your Mac`
        : $localize`:@@place.computer:This computer`;
    case 'worker':
      return $localize`:@@place.worker:Another computer`;
    case undefined:
      return null;
  }
}
