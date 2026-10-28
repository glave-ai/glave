import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { ThreadId } from '@glave/client';

import { NO_STEPS, type Steps, stepping, stepsOf } from '../lib/runs';
import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/**
 * What the server says of the open thread's steps (`thread/steps`), read from its log: each
 * run of calls in a line, and each turn's reply. Read once the thread has opened, if it has made
 * a call, and again each time a call is asked for or ends and a turn ends. Until it is read, and
 * when it cannot be, each run's line keeps its room, unseen, and every message shows plainly.
 * Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class StepsService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  /** As last read; starts again from nothing in each thread. */
  private readonly told = linkedSignal<ThreadId | null, Steps>({
    source: this.thread.thread,
    computation: () => NO_STEPS,
  });
  /** What the steps are read again on, once the thread has opened. */
  private readonly stepping = computed(() =>
    this.thread.opening() ? null : stepping(this.thread.turns()),
  );
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  readonly steps = this.told.asReadonly();

  constructor() {
    effect(() => {
      const thread = this.thread.thread();
      const stepping = this.stepping();
      untracked(() => {
        if (thread !== null && stepping !== null) {
          void this.refresh(thread);
        }
      });
    });
  }

  /** Reads what the server says of `thread`'s steps. */
  private async refresh(thread: ThreadId): Promise<void> {
    this.latest += 1;
    const ticket = this.latest;
    try {
      const { steps } = await this.session.call('thread/steps', { thread });
      if (ticket === this.latest && this.thread.thread() === thread) {
        this.told.set(stepsOf(steps));
      }
    } catch {
      // The steps only add to the thread, which stands without them; the next call asks again.
    }
  }
}
