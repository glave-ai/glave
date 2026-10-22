import { Service, inject, signal } from '@angular/core';
import { ApiError, type ThreadId, type TurnStartParams } from '@glave/client';

import { idempotencyKey } from '../lib/idempotency';
import { SessionService } from './session.service';

/** A thread's first turn as the new chat sent it. */
export interface FirstTurn {
  readonly params: TurnStartParams;
  readonly key: string;
  /** Why it did not go: the server's words, or `lost` when no answer came; else `null`. */
  readonly failed: { readonly said: string } | 'lost' | null;
}

/**
 * The first turn of each thread the new chat starts, sent as the person sends it, so the thread
 * opens at once. Kept in this tab's memory only, never in the page's history, so nothing is sent
 * again when a page is returned to. The thread's page shows it until the server's record of the
 * message arrives, and, when it did not go, why; sent again, it keeps its idempotency key, so
 * the server starts the turn once.
 */
@Service()
export class FirstTurnService {
  private readonly session = inject(SessionService);
  private readonly turns = signal<ReadonlyMap<ThreadId, FirstTurn>>(new Map());

  of(thread: ThreadId): FirstTurn | null {
    return this.turns().get(thread) ?? null;
  }

  /** Sends `params` with a new key, letting go of the first turns the server had. */
  start(params: TurnStartParams): void {
    this.turns.update((turns) => new Map([...turns].filter(([, turn]) => turn.failed !== null)));
    void this.send({ params, key: idempotencyKey(), failed: null });
  }

  /** Sends `thread`'s first turn again, with its key, after it did not go. */
  again(thread: ThreadId): void {
    const turn = this.of(thread);
    if (turn !== null && turn.failed !== null) {
      void this.send({ ...turn, failed: null });
    }
  }

  /** Sends `turn` and waits only for the server's first word back, which says it has it. */
  private async send(turn: FirstTurn): Promise<void> {
    const keep = (kept: FirstTurn): void => {
      this.turns.update((turns) => new Map(turns).set(kept.params.thread, kept));
    };
    keep(turn);
    const controller = new AbortController();
    const params = { ...turn.params, _meta: { idempotency_key: turn.key } };
    try {
      await this.session.stream('turn/start', params, controller.signal).next();
    } catch (error: unknown) {
      keep({ ...turn, failed: error instanceof ApiError ? { said: error.message } : 'lost' });
    } finally {
      // The turn goes on in the server, and the thread's page follows it there.
      controller.abort();
    }
  }
}
