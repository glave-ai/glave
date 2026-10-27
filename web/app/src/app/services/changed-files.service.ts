import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { ChangedFile, ThreadChanges, ThreadId } from '@glave/client';

import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/**
 * What the open thread changed in files (`thread/changes`), as the server reads it from the
 * thread's log: the thread's files, each turn's by the `seq` of its start, and the server's
 * sentence about what they leave out. Read once the thread may have changed a file, and again
 * each time a call that can change one ends. While they are read, and when they cannot be, the
 * thread shows none. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class ChangedFilesService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  /** Starts again from nothing in each thread. */
  private readonly changes = linkedSignal<ThreadId | null, ThreadChanges | null>({
    source: this.thread.thread,
    computation: () => null,
  });
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  /** Every file the thread changed, in the order first changed. */
  readonly files = computed<readonly ChangedFile[]>(() => this.changes()?.files ?? []);
  /** The files each turn changed, by the `seq` of its start. */
  readonly byTurn = computed<ReadonlyMap<number, readonly ChangedFile[]>>(
    () => new Map((this.changes()?.turns ?? []).map((turn) => [turn.turn, turn.files])),
  );
  /** What the files leave out, in the server's sentence; empty until it is read. */
  readonly note = computed(() => this.changes()?.note ?? '');

  constructor() {
    effect(() => {
      const thread = this.thread.thread();
      const changing = this.thread.changing();
      if (thread !== null && changing !== null) {
        untracked(() => void this.refresh());
      }
    });
  }

  /** Reads what the thread changed. */
  async refresh(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    this.latest += 1;
    const ticket = this.latest;
    try {
      const { changes } = await this.session.call('thread/changes', { thread });
      if (ticket === this.latest && this.thread.thread() === thread) {
        this.changes.set(changes);
      }
    } catch {
      // What changed only adds to the thread, which stands without it; the next change asks again.
    }
  }
}
