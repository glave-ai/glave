import { Service, computed, effect, inject, linkedSignal, signal, untracked } from '@angular/core';
import type { PlanInfo, ThreadId } from '@glave/client';

import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/**
 * The open thread's plan (`thread/plan`), as the server reads it from the thread's log: read
 * once the thread has one, and again at each change to it, so a plan being built ticks off as
 * the model keeps it; and the person's edit of a proposal (`plan/edit`). Building it is a turn,
 * which the thread follows (`ThreadService.build`). Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class PlanService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  /** Starts again from nothing in each thread. */
  private readonly read = linkedSignal<ThreadId | null, PlanInfo | null>({
    source: this.thread.thread,
    computation: () => null,
  });
  private readonly sending = signal(false);
  private readonly failed = signal<string | null>(null);
  /** Whether a read has answered, or failed, in this thread. */
  private readonly answered = linkedSignal<ThreadId | null, boolean>({
    source: this.thread.thread,
    computation: () => false,
  });
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  /** The thread's plan, or `null` while it has none or it is being read. */
  readonly plan = this.read.asReadonly();
  /** Whether what the thread's history calls for has been read, or there is nothing to read. */
  readonly known = computed(() => this.thread.planning() === null || this.answered());
  /** Whether the person's edit is on its way. */
  readonly busy = this.sending.asReadonly();
  /** Why the edit was refused, in the server's words. */
  readonly error = this.failed.asReadonly();

  constructor() {
    effect(() => {
      const thread = this.thread.thread();
      const planning = this.thread.planning();
      if (thread !== null && planning !== null) {
        untracked(() => void this.refresh());
      }
    });
  }

  /** Reads the thread's plan. */
  async refresh(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    this.latest += 1;
    const ticket = this.latest;
    try {
      const { plan } = await this.session.call('thread/plan', { thread });
      if (ticket === this.latest && this.thread.thread() === thread) {
        this.read.set(plan ?? null);
      }
    } catch {
      // The plan stands as it was read last; its next change reads it again.
    }
    if (ticket === this.latest && this.thread.thread() === thread) {
      this.answered.set(true);
    }
  }

  /** Puts the person's `steps` in place of the proposal's, as the server keeps them. */
  async edit(steps: readonly string[]): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null || this.sending()) {
      return;
    }
    this.sending.set(true);
    this.failed.set(null);
    this.latest += 1;
    try {
      const { plan } = await this.session.call('plan/edit', { thread, steps });
      if (this.thread.thread() === thread) {
        this.read.set(plan ?? null);
      }
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.sending.set(false);
    }
  }
}
