import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { ReplySources, ThreadId } from '@glave/client';

import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/**
 * What the open thread's replies drew on from the web (`thread/sources`), by the `seq` of each
 * reply, as the server reads it from the thread's log: read once the thread has read a page,
 * and again each time a turn ends after one did. While they are read, and when they cannot be,
 * the replies show without them. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class SourcesService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  /** Starts again from nothing in each thread. */
  private readonly sources = linkedSignal<ThreadId | null, readonly ReplySources[]>({
    source: this.thread.thread,
    computation: () => [],
  });
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  /** Each reply's sources, by the `seq` of the reply. */
  readonly byReply = computed<ReadonlyMap<number, ReplySources>>(
    () => new Map(this.sources().map((sourced) => [sourced.reply, sourced])),
  );

  constructor() {
    effect(() => {
      const thread = this.thread.thread();
      const sourcing = this.thread.sourcing();
      if (thread !== null && sourcing !== null) {
        untracked(() => void this.refresh());
      }
    });
  }

  /** Reads what the thread's replies drew on. */
  async refresh(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    this.latest += 1;
    const ticket = this.latest;
    try {
      const { sources } = await this.session.call('thread/sources', { thread });
      if (ticket === this.latest && this.thread.thread() === thread) {
        this.sources.set(sources);
      }
    } catch {
      // Sources only add to a reply, which stands without them; the next turn's end asks again.
    }
  }
}
