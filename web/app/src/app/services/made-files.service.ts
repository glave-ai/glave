import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { ThreadId, TurnFiles } from '@glave/client';

import { type MadeLater, madeCount, madeLater } from '../lib/made';
import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/**
 * The files the open thread's turns made for the person (`thread/files`), by the `seq` of each
 * turn's start, with what each turn's files leave out, as the server reads them from the
 * thread's log: read once the thread's history says what a turn made, and again each time it
 * says more (another turn's end, or a later look-over). Until they are first known, `null`, so a
 * turn keeps its cards' room from what its log says it made; when they cannot be read, the turns
 * show without them, and keep no room. Each thread view provides its own.
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
  /** What later look-overs said each turn made, from the thread's log. */
  readonly later = computed<ReadonlyMap<number, MadeLater>>(() => madeLater(this.thread.events()));
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;
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
      const making = madeCount(this.thread.events());
      if (thread !== null && making > 0) {
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
        this.failed.set(false);
      }
    } catch {
      // The files only add to a turn, which stands without them, and without their room; the next
      // turn's end asks again.
      if (ticket === this.latest && this.thread.thread() === thread) {
        this.failed.set(true);
      }
    }
  }
}
