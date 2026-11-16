import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { MadeFile, ThreadId, TurnFiles } from '@glave/client';

import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/**
 * The files the open thread's turns made for the person (`thread/files`), by the `seq` of each
 * turn's start, as the server reads them from the thread's log: read once the thread has run a
 * call that can make one, and again each time one ends. While they are read, and when they
 * cannot be, the turns show without them. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class MadeFilesService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  /** Starts again from nothing in each thread. */
  private readonly made = linkedSignal<ThreadId | null, readonly TurnFiles[]>({
    source: this.thread.thread,
    computation: () => [],
  });
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  /** The files each turn made, by the `seq` of its start. */
  readonly byTurn = computed<ReadonlyMap<number, readonly MadeFile[]>>(
    () => new Map(this.made().map((turn) => [turn.turn, turn.files])),
  );

  constructor() {
    effect(() => {
      const thread = this.thread.thread();
      const changing = this.thread.changing();
      if (thread !== null && changing !== null) {
        untracked(() => void this.refresh());
      }
    });
  }

  /** Reads the files the thread's turns made. */
  async refresh(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    this.latest += 1;
    const ticket = this.latest;
    try {
      const { made } = await this.session.call('thread/files', { thread });
      if (ticket === this.latest && this.thread.thread() === thread) {
        this.made.set(made);
      }
    } catch {
      // The files only add to a turn, which stands without them; the next call's end asks again.
    }
  }
}
