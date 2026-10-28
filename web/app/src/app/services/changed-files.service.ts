import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { ChangedFile, ThreadChanges, ThreadId } from '@glave/client';

import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/** What a thread changed when its history holds no call that can change a file: nothing. */
const NONE: ThreadChanges = { files: [], turns: [], note: '' };

/**
 * What the open thread changed in files (`thread/changes`), as the server reads it from the
 * thread's log: the thread's files, each turn's by the `seq` of its start, and the server's
 * sentence about what they leave out. Read once the thread has opened, if it may have changed a
 * file, and again each time a call that can change one ends. While they are read, and when they
 * cannot be, the thread shows none; until they are first known, a turn's are not known either,
 * so what the thread had changed as it opened is told from what changes as the person watches.
 * Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class ChangedFilesService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  /** Once known; starts again from nothing in each thread. */
  private readonly changes = linkedSignal<ThreadId | null, ThreadChanges | null>({
    source: this.thread.thread,
    computation: () => null,
  });
  /** Whether a read has answered, or failed, in this thread. */
  private readonly answered = linkedSignal<ThreadId | null, boolean>({
    source: this.thread.thread,
    computation: () => false,
  });
  /** Counts the reads begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  /** Every file the thread changed, in the order first changed. */
  readonly files = computed<readonly ChangedFile[]>(() => this.changes()?.files ?? []);
  /**
   * The files each turn changed, by the `seq` of its start; `null` until they are first known:
   * while the thread opens and they are read, and when that read fails.
   */
  readonly byTurn = computed<ReadonlyMap<number, readonly ChangedFile[]> | null>(() => {
    const changes = this.changes();
    return changes === null ? null : new Map(changes.turns.map((turn) => [turn.turn, turn.files]));
  });
  /** What the files leave out, in the server's sentence; empty until it is read. */
  readonly note = computed(() => this.changes()?.note ?? '');
  /** Whether what the thread's history calls for has been read, or there is nothing to read. */
  readonly known = computed(() => this.thread.changing() === null || this.answered());

  constructor() {
    effect(() => {
      const thread = this.thread.thread();
      const changing = this.thread.changing();
      const opening = this.thread.opening();
      untracked(() => {
        if (thread === null || opening) {
          return;
        }
        if (changing !== null) {
          void this.refresh();
        } else if (this.changes() === null) {
          this.changes.set(NONE);
        }
      });
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
    if (ticket === this.latest && this.thread.thread() === thread) {
      this.answered.set(true);
    }
  }
}
