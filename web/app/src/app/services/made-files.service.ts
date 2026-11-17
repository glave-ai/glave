import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { ThreadId, TurnFiles } from '@glave/client';

import { OneAtATime } from '../lib/reads';
import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/**
 * The files the open thread's turns made for the person (`thread/files`), by the `seq` of each
 * turn's start, with what each turn's files leave out, as the server reads them from the
 * thread's log: read once the thread has opened, if its history says a turn made one, and again
 * each time it says more (another turn's end that made one, or a later look-over), and never on
 * anything else. Until they are first known, `null`, so a turn keeps its cards' room from what its
 * log says it made; when they cannot be read, what was read before stays, and no room is kept.
 * Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class MadeFilesService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  /** Once known; starts again from nothing in each thread. */
  private readonly made = linkedSignal<ThreadId | null, readonly TurnFiles[] | null>({
    source: this.thread.thread,
    computation: () => null,
  });
  /** Whether they couldn't be read, the last time they were asked for. */
  readonly failed = linkedSignal<ThreadId | null, boolean>({
    source: this.thread.thread,
    computation: () => false,
  });
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;
  /** Its reads, one at a time. */
  private readonly reads = new OneAtATime(() => this.read());
  /**
   * The files each turn made, and what they leave out, by the `seq` of its start; `null` until
   * first known.
   */
  readonly byTurn = computed<ReadonlyMap<number, TurnFiles> | null>(() => {
    const made = this.made();
    return made === null ? null : new Map(made.map((turn) => [turn.turn, turn]));
  });

  constructor() {
    effect(() => {
      const thread = this.thread.thread();
      const making = this.thread.made.changed();
      untracked(() => {
        if (thread !== null && making !== null) {
          void this.refresh();
        }
      });
    });
  }

  /**
   * Reads the files the thread's turns made: at once, or, while a read is in flight, once more
   * after it, however often it was asked meanwhile.
   */
  async refresh(): Promise<void> {
    return this.reads.run();
  }

  /** Reads the files the thread's turns made, once. */
  private async read(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    this.latest += 1;
    const ticket = this.latest;
    // The files only add to a turn, which stands without them; the next that says more asks again.
    const read = await this.session.call('thread/files', { thread }).catch(() => null);
    if (ticket !== this.latest || this.thread.thread() !== thread) {
      return;
    }
    if (read === null) {
      this.failed.set(true);
    } else {
      this.made.set(read.made);
      this.failed.set(false);
    }
  }
}
